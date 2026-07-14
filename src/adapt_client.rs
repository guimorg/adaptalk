use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::future::Future;
use thiserror::Error;
use tokio::sync::OnceCell;

use crate::config::AdaptConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryResponse {
    pub content: Vec<rmcp::model::Content>,
    pub structured_content: Option<Value>,
}

// Keep this list empty until Adapt documents and verifies a non-mutating
// capability. MCP annotations are hints supplied by the remote server, not a
// sufficient authorization boundary on their own.
const VERIFIED_READ_ONLY_CAPABILITIES: &[&str] = &[];

#[derive(Debug, Error)]
pub enum AdaptClientError {
    #[error("Adapt authentication was rejected by the server")]
    AuthenticationRejected,
    #[error("Adapt capability `{0}` is not verified as read-only")]
    CapabilityNotReadOnly(String),
    #[error("Adapt capability `ask_adapt` requires --allow-unverified-ask-adapt")]
    AskAdaptOptInRequired,
    #[error("Adapt capability `{0}` returned an error")]
    CapabilityFailed(String),
    #[error("Adapt has no verified read-only capability available")]
    NoReadOnlyCapability,
    #[error("Adapt endpoint or transport failed: {0}")]
    Transport(String),
}

pub struct AdaptClient {
    service: rmcp::service::RunningService<rmcp::RoleClient, ClientHandlerImpl>,
    credential: String,
    capabilities: CapabilityCache,
}

#[derive(Default)]
struct CapabilityCache {
    tools: OnceCell<Vec<rmcp::model::Tool>>,
}

impl CapabilityCache {
    async fn get_or_try_init<L, F>(
        &self,
        loader: L,
    ) -> Result<&[rmcp::model::Tool], AdaptClientError>
    where
        L: FnOnce() -> F,
        F: Future<Output = Result<Vec<rmcp::model::Tool>, AdaptClientError>>,
    {
        self.tools.get_or_try_init(loader).await.map(Vec::as_slice)
    }
}
#[derive(Default)]
struct ClientHandlerImpl;
impl ClientHandler for ClientHandlerImpl {}

impl AdaptClient {
    pub async fn connect(config: &AdaptConfig) -> Result<Self, AdaptClientError> {
        let transport = StreamableHttpClientTransport::from_config(transport_config(config));
        let service = ClientHandlerImpl
            .serve(transport)
            .await
            .map_err(|e| map_transport_error(e, &config.bearer_token))?;
        Ok(Self {
            service,
            credential: config.bearer_token.clone(),
            capabilities: CapabilityCache::default(),
        })
    }

    pub async fn discover_capabilities(&self) -> Result<Vec<Capability>, AdaptClientError> {
        let tools = self.cached_tools().await?;
        Ok(tools
            .iter()
            .map(|tool| Capability {
                name: tool.name.to_string(),
                description: tool.description.as_ref().map(|d| d.to_string()),
            })
            .collect())
    }

    /// Return only capabilities explicitly verified as read-only by Adapt.
    ///
    /// A server annotation is required, but it is not sufficient: the name
    /// must also be present in the Adapt-specific verification list below.
    pub async fn discover_read_only_capabilities(
        &self,
    ) -> Result<Vec<Capability>, AdaptClientError> {
        let tools = self.cached_tools().await?;
        Ok(tools
            .iter()
            .filter(|tool| is_verified_read_only(tool))
            .map(|tool| Capability {
                name: tool.name.to_string(),
                description: tool.description.as_ref().map(|d| d.to_string()),
            })
            .collect())
    }

    /// Invoke a selected read-only capability with the user's prompt.
    pub async fn query(
        &self,
        capability: &str,
        prompt: &str,
    ) -> Result<QueryResponse, AdaptClientError> {
        let tools = self.cached_tools().await?;
        let Some(tool) = tools.iter().find(|tool| tool.name == capability) else {
            return Err(AdaptClientError::CapabilityNotReadOnly(
                capability.to_owned(),
            ));
        };
        if !is_verified_read_only(tool) {
            return Err(AdaptClientError::CapabilityNotReadOnly(
                capability.to_owned(),
            ));
        }

        self.invoke_tool(capability, prompt).await
    }

    /// Invoke Adapt's unverified `ask_adapt` capability for development only.
    ///
    /// This seam is intentionally narrower than [`Self::query`]: callers must
    /// opt in explicitly, and no arbitrary unverified capability can be
    /// selected through it.
    pub async fn query_ask_adapt(
        &self,
        prompt: &str,
        allow_unverified: bool,
    ) -> Result<QueryResponse, AdaptClientError> {
        ensure_ask_adapt_opt_in(allow_unverified)?;

        let tools = self.cached_tools().await?;
        if !is_allowed_unverified_capability("ask_adapt")
            || !tools.iter().any(|tool| tool.name == "ask_adapt")
        {
            return Err(AdaptClientError::CapabilityNotReadOnly(
                "ask_adapt".to_owned(),
            ));
        }
        self.invoke_tool("ask_adapt", prompt).await
    }

    /// Submit a prompt through the only available verified read-only capability.
    ///
    /// Keeping capability selection here prevents the terminal layer from making
    /// policy decisions or accidentally invoking an unverified tool.
    pub async fn query_read_only(&self, prompt: &str) -> Result<QueryResponse, AdaptClientError> {
        let tools = self.cached_tools().await?;
        let tool = tools
            .iter()
            .find(|tool| is_verified_read_only(tool))
            .ok_or(AdaptClientError::NoReadOnlyCapability)?;
        self.invoke_tool(tool.name.as_ref(), prompt).await
    }

    async fn cached_tools(&self) -> Result<&[rmcp::model::Tool], AdaptClientError> {
        self.capabilities
            .get_or_try_init(|| async {
                self.service
                    .peer()
                    .list_all_tools()
                    .await
                    .map_err(|e| map_transport_error(e, &self.credential))
            })
            .await
    }

    async fn invoke_tool(
        &self,
        capability: &str,
        prompt: &str,
    ) -> Result<QueryResponse, AdaptClientError> {
        let mut arguments = Map::new();
        arguments.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
        let result: CallToolResult = self
            .service
            .peer()
            .call_tool(CallToolRequestParams {
                meta: None,
                name: capability.to_owned().into(),
                arguments: Some(arguments),
                task: None,
            })
            .await
            .map_err(|e| map_transport_error(e, &self.credential))?;
        if result.is_error == Some(true) {
            return Err(AdaptClientError::CapabilityFailed(capability.to_owned()));
        }
        Ok(QueryResponse {
            content: result.content,
            structured_content: result.structured_content,
        })
    }
}

fn is_verified_read_only(tool: &rmcp::model::Tool) -> bool {
    VERIFIED_READ_ONLY_CAPABILITIES.contains(&tool.name.as_ref())
        && tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            == Some(true)
}

fn transport_config(config: &AdaptConfig) -> StreamableHttpClientTransportConfig {
    StreamableHttpClientTransportConfig::with_uri(config.endpoint.clone())
        // RMCP adds the `Bearer ` prefix; this value must be the raw session token.
        .auth_header(config.bearer_token.clone())
}

fn map_transport_error(error: impl std::fmt::Display, credential: &str) -> AdaptClientError {
    let text = error.to_string();
    let normalized = text.to_ascii_lowercase();
    if normalized.contains("auth required")
        || normalized.contains("insufficient scope")
        || normalized.contains("status code: 401")
        || normalized.contains("status code: 403")
    {
        AdaptClientError::AuthenticationRejected
    } else {
        AdaptClientError::Transport(sanitize_transport_error(&text, credential))
    }
}

fn sanitize_transport_error(text: &str, credential: &str) -> String {
    if credential.is_empty() {
        return text.to_owned();
    }
    text.replace(credential, "[redacted credential]")
}

fn ensure_ask_adapt_opt_in(allow_unverified: bool) -> Result<(), AdaptClientError> {
    if allow_unverified {
        Ok(())
    } else {
        Err(AdaptClientError::AskAdaptOptInRequired)
    }
}

fn is_allowed_unverified_capability(capability: &str) -> bool {
    capability == "ask_adapt"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Barrier;

    #[test]
    fn transport_errors_redact_the_credential() {
        let error = map_transport_error(
            "request failed for Bearer super-secret-token",
            "super-secret-token",
        );
        let AdaptClientError::Transport(message) = error else {
            panic!("expected transport error");
        };
        assert!(!message.contains("super-secret-token"));
        assert!(message.contains("[redacted credential]"));
    }

    #[test]
    fn authentication_errors_are_classified_consistently() {
        assert!(matches!(
            map_transport_error("server returned status code: 401", "secret"),
            AdaptClientError::AuthenticationRejected
        ));
        assert!(matches!(
            map_transport_error("Auth required", "secret"),
            AdaptClientError::AuthenticationRejected
        ));
        assert!(matches!(
            map_transport_error("server returned status code: 403", ""),
            AdaptClientError::AuthenticationRejected
        ));
    }

    #[test]
    fn transport_config_passes_raw_token_to_rmcp() {
        let config = AdaptConfig {
            bearer_token: "session-token".into(),
            endpoint: "https://app.adapt.com/mcp".into(),
            source: "/tmp/config.toml".into(),
        };
        assert_eq!(
            transport_config(&config).auth_header.as_deref(),
            Some("session-token")
        );
    }

    #[test]
    fn read_only_policy_requires_an_explicit_true_annotation() {
        let mut tool = rmcp::model::Tool::new("safe", "safe query", Map::new());
        assert!(!is_verified_read_only(&tool));
        tool.annotations = Some(rmcp::model::ToolAnnotations {
            read_only_hint: Some(false),
            ..Default::default()
        });
        assert!(!is_verified_read_only(&tool));
        tool.annotations.as_mut().unwrap().read_only_hint = Some(true);
        assert!(!is_verified_read_only(&tool));
    }

    #[test]
    fn ask_adapt_is_rejected_even_if_the_server_claims_it_is_read_only() {
        let tool = rmcp::model::Tool::new("ask_adapt", "Adapt query", Map::new()).annotate(
            rmcp::model::ToolAnnotations {
                read_only_hint: Some(true),
                ..Default::default()
            },
        );
        assert!(!is_verified_read_only(&tool));
    }

    #[test]
    fn ask_adapt_requires_explicit_opt_in() {
        assert!(matches!(
            ensure_ask_adapt_opt_in(false),
            Err(AdaptClientError::AskAdaptOptInRequired)
        ));
        assert!(ensure_ask_adapt_opt_in(true).is_ok());
    }

    #[test]
    fn arbitrary_unverified_capabilities_are_not_allowed_by_ask_adapt_policy() {
        assert!(!is_allowed_unverified_capability("other_tool"));
        assert!(is_allowed_unverified_capability("ask_adapt"));
    }

    #[test]
    fn opt_in_error_does_not_include_credentials() {
        let error = ensure_ask_adapt_opt_in(false).unwrap_err().to_string();
        assert!(!error.contains("secret"));
        assert!(error.contains("--allow-unverified-ask-adapt"));
    }

    fn test_tool(name: &str) -> rmcp::model::Tool {
        rmcp::model::Tool::new(name.to_owned(), format!("{name} query"), Map::new())
    }

    #[tokio::test]
    async fn cache_shares_one_load_across_reads() {
        let cache = CapabilityCache::default();
        let loads = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let loads = Arc::clone(&loads);
            let tools = cache
                .get_or_try_init(|| async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![test_tool("safe")])
                })
                .await
                .unwrap();
            assert_eq!(tools[0].name, "safe");
        }

        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_first_access_initializes_cache_once() {
        let cache = Arc::new(CapabilityCache::default());
        let loads = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let mut tasks = Vec::new();

        for _ in 0..8 {
            let cache = Arc::clone(&cache);
            let loads = Arc::clone(&loads);
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                cache
                    .get_or_try_init(|| async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok(vec![test_tool("safe")])
                    })
                    .await
                    .unwrap()
                    .len()
            }));
        }

        for task in tasks {
            assert_eq!(task.await.unwrap(), 1);
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_load_does_not_poison_cache() {
        let cache = CapabilityCache::default();
        let loads = Arc::new(AtomicUsize::new(0));

        let first = {
            let loads = Arc::clone(&loads);
            cache
                .get_or_try_init(|| async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Err(AdaptClientError::Transport("temporary failure".into()))
                })
                .await
        };
        assert!(first.is_err());

        let second = {
            let loads = Arc::clone(&loads);
            cache
                .get_or_try_init(|| async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![test_tool("safe")])
                })
                .await
        };
        assert_eq!(second.unwrap()[0].name, "safe");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }
}
