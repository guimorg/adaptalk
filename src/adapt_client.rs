use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

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
        })
    }

    pub async fn discover_capabilities(&self) -> Result<Vec<Capability>, AdaptClientError> {
        let tools = self
            .service
            .peer()
            .list_all_tools()
            .await
            .map_err(|e| map_transport_error(e, &self.credential))?;
        Ok(tools
            .into_iter()
            .map(|tool| Capability {
                name: tool.name.to_string(),
                description: tool.description.map(|d| d.to_string()),
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
        let tools = self
            .service
            .peer()
            .list_all_tools()
            .await
            .map_err(|e| map_transport_error(e, &self.credential))?;
        Ok(tools
            .into_iter()
            .filter(is_verified_read_only)
            .map(|tool| Capability {
                name: tool.name.to_string(),
                description: tool.description.map(|d| d.to_string()),
            })
            .collect())
    }

    /// Invoke a selected read-only capability with the user's prompt.
    pub async fn query(
        &self,
        capability: &str,
        prompt: &str,
    ) -> Result<QueryResponse, AdaptClientError> {
        let tools = self
            .service
            .peer()
            .list_all_tools()
            .await
            .map_err(|e| map_transport_error(e, &self.credential))?;
        let Some(tool) = tools.into_iter().find(|tool| tool.name == capability) else {
            return Err(AdaptClientError::CapabilityNotReadOnly(
                capability.to_owned(),
            ));
        };
        if !is_verified_read_only(&tool) {
            return Err(AdaptClientError::CapabilityNotReadOnly(
                capability.to_owned(),
            ));
        }

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

    /// Submit a prompt through the only available verified read-only capability.
    ///
    /// Keeping capability selection here prevents the terminal layer from making
    /// policy decisions or accidentally invoking an unverified tool.
    pub async fn query_read_only(&self, prompt: &str) -> Result<QueryResponse, AdaptClientError> {
        let capabilities = self.discover_read_only_capabilities().await?;
        let capability = capabilities
            .first()
            .ok_or(AdaptClientError::NoReadOnlyCapability)?;
        self.query(&capability.name, prompt).await
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
