use rmcp::{
    ClientHandler, ServiceExt,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Serialize;
use thiserror::Error;

use crate::config::AdaptConfig;

/// Contract-verified read-only tools. New names require explicit review before exposure.
pub const READ_ONLY_ALLOWLIST: &[&str] = &["search", "fetch"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisedCapability {
    pub name: String,
    pub description: Option<String>,
    pub annotations: Option<CapabilityAnnotations>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAnnotations {
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
}

#[derive(Debug, Error)]
pub enum AdaptClientError {
    #[error("Adapt authentication was rejected by the server")]
    AuthenticationRejected,
    #[error("Adapt endpoint or transport failed: {0}")]
    Transport(String),
    #[error("Adapt returned an invalid capability list: {0}")]
    CapabilityRejected(String),
}

pub fn filter_capabilities(
    tools: impl IntoIterator<Item = AdvertisedCapability>,
) -> Result<Vec<Capability>, AdaptClientError> {
    let mut safe = Vec::new();
    for tool in tools {
        if !READ_ONLY_ALLOWLIST.contains(&tool.name.as_str()) {
            continue;
        }
        let Some(a) = tool.annotations else {
            return Err(AdaptClientError::CapabilityRejected(format!(
                "'{name}' has no safety metadata",
                name = tool.name
            )));
        };
        if a.read_only_hint != Some(true) || a.destructive_hint == Some(true) {
            return Err(AdaptClientError::CapabilityRejected(format!(
                "'{name}' has ambiguous safety metadata",
                name = tool.name
            )));
        }
        safe.push(Capability {
            name: tool.name,
            description: tool.description,
        });
    }
    Ok(safe)
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
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(config.endpoint.clone())
                .auth_header(format!("Bearer {}", config.bearer_token)),
        );
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
        filter_capabilities(tools.into_iter().map(|tool| AdvertisedCapability {
            name: tool.name.to_string(),
            description: tool.description.map(|d| d.to_string()),
            annotations: tool.annotations.map(|a| CapabilityAnnotations {
                read_only_hint: a.read_only_hint,
                destructive_hint: a.destructive_hint,
            }),
        }))
    }
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
    fn c(name: &str, ro: Option<bool>, destructive: Option<bool>) -> AdvertisedCapability {
        AdvertisedCapability {
            name: name.into(),
            description: None,
            annotations: Some(CapabilityAnnotations {
                read_only_hint: ro,
                destructive_hint: destructive,
            }),
        }
    }
    #[test]
    fn allowlist_and_metadata_fail_closed() {
        assert_eq!(
            filter_capabilities([
                c("search", Some(true), Some(false)),
                c("mutate", Some(true), Some(false))
            ])
            .unwrap()
            .len(),
            1
        );
        assert!(filter_capabilities([c("search", None, Some(false))]).is_err());
        assert!(filter_capabilities([c("search", Some(true), Some(true))]).is_err());
    }
    #[test]
    fn unknown_is_hidden() {
        assert!(
            filter_capabilities([c("unknown", None, None)])
                .unwrap()
                .is_empty()
        );
    }

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
}
