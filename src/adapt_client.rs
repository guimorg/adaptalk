use rmcp::{
    ClientHandler, ServiceExt,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Serialize;
use thiserror::Error;

use crate::config::AdaptConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Error)]
pub enum AdaptClientError {
    #[error("Adapt authentication was rejected by the server")]
    AuthenticationRejected,
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
}
