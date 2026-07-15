//! Credential validation and error classification for Adapt authentication.

use std::time::Duration;
use thiserror::Error;

use crate::config::AdaptConfig;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error(
        "Adapt authentication was rejected by the server; refresh bearer_token in ~/.adapt/config.toml and retry"
    )]
    AuthenticationRejected,
    #[error("Adapt endpoint or transport failed: {0}")]
    Transport(String),
}

/// Validate that the credential can authenticate with Adapt before attempting MCP connection.
///
/// Adapt returns a JSON error body without a `WWW-Authenticate` header for an
/// invalid session. RMCP 0.16 then attempts to decode that body as JSON-RPC
/// during initialization, hiding the actionable authentication failure. A
/// lightweight authenticated GET lets us report the rejected credential first.
pub async fn validate_credentials(config: &AdaptConfig) -> Result<(), AuthError> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| map_transport_error(error, &config.bearer_token))?
        .get(&config.endpoint)
        .bearer_auth(&config.bearer_token)
        .send()
        .await
        .map_err(|error| map_transport_error(error, &config.bearer_token))?;

    if is_authentication_rejection(response.status()) {
        Err(AuthError::AuthenticationRejected)
    } else {
        Ok(())
    }
}

/// Classify a transport error as authentication failure or generic transport issue.
pub fn map_transport_error(error: impl std::fmt::Display, credential: &str) -> AuthError {
    let text = error.to_string();
    let normalized = text.to_ascii_lowercase();
    if normalized.contains("auth required")
        || normalized.contains("insufficient scope")
        || normalized.contains("status code: 401")
        || normalized.contains("status code: 403")
    {
        AuthError::AuthenticationRejected
    } else {
        AuthError::Transport(sanitize_transport_error(&text, credential))
    }
}

/// Remove credential values from error messages to prevent accidental logging.
pub fn sanitize_transport_error(text: &str, credential: &str) -> String {
    if credential.is_empty() {
        return text.to_owned();
    }
    text.replace(credential, "[redacted credential]")
}

fn is_authentication_rejection(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    )
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
        let AuthError::Transport(message) = error else {
            panic!("expected transport error");
        };
        assert!(!message.contains("super-secret-token"));
        assert!(message.contains("[redacted credential]"));
    }

    #[test]
    fn authentication_errors_are_classified_consistently() {
        assert!(matches!(
            map_transport_error("server returned status code: 401", "secret"),
            AuthError::AuthenticationRejected
        ));
        assert!(matches!(
            map_transport_error("Auth required", "secret"),
            AuthError::AuthenticationRejected
        ));
        assert!(matches!(
            map_transport_error("server returned status code: 403", ""),
            AuthError::AuthenticationRejected
        ));
    }

    #[test]
    fn preflight_recognizes_headerless_authentication_rejections() {
        assert!(is_authentication_rejection(
            reqwest::StatusCode::UNAUTHORIZED
        ));
        assert!(is_authentication_rejection(reqwest::StatusCode::FORBIDDEN));
        assert!(!is_authentication_rejection(
            reqwest::StatusCode::METHOD_NOT_ALLOWED
        ));
    }
}
