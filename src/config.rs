use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

pub const DEFAULT_ADAPT_ENDPOINT: &str = "https://mcp.adapt.ai/mcp";

#[derive(Clone, PartialEq, Eq)]
pub struct AdaptConfig {
    pub bearer_token: String,
    pub endpoint: String,
    pub source: PathBuf,
}

impl std::fmt::Debug for AdaptConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdaptConfig")
            .field("bearer_token", &"[redacted]")
            .field("endpoint", &self.endpoint)
            .field("source", &self.source)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Adapt configuration is missing at {path}; create it with a bearer_token")]
    Missing { path: PathBuf },
    #[error("Adapt configuration at {path} is malformed TOML")]
    Malformed { path: PathBuf },
    #[error("Adapt configuration at {path} does not contain a bearer_token")]
    MissingToken { path: PathBuf },
    #[error("Adapt configuration at {path} has an invalid endpoint")]
    InvalidEndpoint { path: PathBuf },
    #[error("could not read Adapt configuration at {path}")]
    Read { path: PathBuf },
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    bearer_token: Option<String>,
    endpoint: Option<String>,
}

pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|p| p.join(".adapt/config.toml"))
        .ok_or_else(|| ConfigError::Missing {
            path: PathBuf::from("~/.adapt/config.toml"),
        })
}

pub fn load() -> Result<AdaptConfig, ConfigError> {
    load_from(default_config_path()?)
}

pub fn load_from(path: impl AsRef<Path>) -> Result<AdaptConfig, ConfigError> {
    let path = path.as_ref().to_path_buf();
    let text = fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ConfigError::Missing { path: path.clone() }
        } else {
            ConfigError::Read { path: path.clone() }
        }
    })?;
    let parsed: FileConfig =
        toml::from_str(&text).map_err(|_| ConfigError::Malformed { path: path.clone() })?;
    let token = parsed
        .bearer_token
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ConfigError::MissingToken { path: path.clone() })?;
    let endpoint = parsed
        .endpoint
        .unwrap_or_else(|| DEFAULT_ADAPT_ENDPOINT.to_owned());
    if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
        return Err(ConfigError::InvalidEndpoint { path });
    }
    Ok(AdaptConfig {
        bearer_token: token,
        endpoint,
        source: path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("adapt-tui-{name}-{}.toml", std::process::id()))
    }
    #[test]
    fn defaults_endpoint() {
        let p = path("default");
        fs::write(&p, "bearer_token = 'secret'").unwrap();
        let c = load_from(&p).unwrap();
        assert_eq!(c.endpoint, DEFAULT_ADAPT_ENDPOINT);
        assert!(!format!("{c:?}").contains("secret"));
        let _ = fs::remove_file(p);
    }
    #[test]
    fn override_endpoint() {
        let p = path("override");
        fs::write(
            &p,
            "bearer_token='s'\nendpoint='https://staging.example/mcp'",
        )
        .unwrap();
        assert_eq!(
            load_from(&p).unwrap().endpoint,
            "https://staging.example/mcp"
        );
        let _ = fs::remove_file(p);
    }
    #[test]
    fn missing_and_malformed_are_distinct() {
        let p = path("missing");
        assert!(matches!(load_from(&p), Err(ConfigError::Missing { .. })));
        fs::write(&p, "=").unwrap();
        assert!(matches!(load_from(&p), Err(ConfigError::Malformed { .. })));
        let _ = fs::remove_file(p);
    }
    #[test]
    fn missing_token_is_distinct() {
        let p = path("token");
        fs::write(&p, "endpoint='https://x'").unwrap();
        assert!(matches!(
            load_from(&p),
            Err(ConfigError::MissingToken { .. })
        ));
        let _ = fs::remove_file(p);
    }
}
