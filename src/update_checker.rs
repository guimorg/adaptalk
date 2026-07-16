use semver::Version;
use serde::Deserialize;
use std::{fmt, time::Duration};
use tokio::sync::mpsc::{self, UnboundedReceiver};

const RELEASES_URL: &str = "https://api.github.com/repos/guimorg/adaptalk/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNotice {
    pub installed: Version,
    pub available: Version,
    pub url: String,
}

#[derive(Debug)]
pub enum UpdateCheckError {
    Http(reqwest::Error),
    Json(serde_json::Error),
    InvalidInstalledVersion,
    InvalidReleaseVersion,
}
impl fmt::Display for UpdateCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "update check failed")
    }
}
impl std::error::Error for UpdateCheckError {}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
}

pub type UpdateResult = Result<Option<UpdateNotice>, UpdateCheckError>;

pub fn spawn() -> UnboundedReceiver<UpdateResult> {
    let (sender, receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .user_agent(concat!("adaptalk/", env!("CARGO_PKG_VERSION")))
            .build();
        let result = match client {
            Ok(client) => check_latest_at(&client, RELEASES_URL).await,
            Err(error) => Err(UpdateCheckError::Http(error)),
        };
        let _ = sender.send(result);
    });
    receiver
}

pub async fn check_latest_at(client: &reqwest::Client, url: &str) -> UpdateResult {
    let installed = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|_| UpdateCheckError::InvalidInstalledVersion)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(UpdateCheckError::Http)?;
    let body = response
        .error_for_status()
        .map_err(UpdateCheckError::Http)?
        .text()
        .await
        .map_err(UpdateCheckError::Http)?;
    parse_release(&body, installed)
}

pub fn parse_release(body: &str, installed: Version) -> UpdateResult {
    let release: Release = serde_json::from_str(body).map_err(UpdateCheckError::Json)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let tag = release.tag_name.trim().trim_start_matches(['v', 'V']);
    let available = Version::parse(tag).map_err(|_| UpdateCheckError::InvalidReleaseVersion)?;
    Ok((available > installed).then_some(UpdateNotice {
        installed,
        available,
        url: release.html_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str, draft: bool, prerelease: bool) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/example/release","draft":{draft},"prerelease":{prerelease}}}"#
        )
    }
    #[test]
    fn newer_equal_older_and_v_prefix() {
        let installed = Version::new(1, 0, 0);
        assert!(
            parse_release(&release("v2.0.0", false, false), installed.clone())
                .unwrap()
                .is_some()
        );
        assert!(
            parse_release(&release("1.0.0", false, false), installed.clone())
                .unwrap()
                .is_none()
        );
        assert!(
            parse_release(&release("0.9.0", false, false), installed)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn drafts_prereleases_and_malformed_input_are_ignored_or_rejected() {
        let v = Version::new(1, 0, 0);
        assert_eq!(
            parse_release(&release("2.0.0", true, false), v.clone()).unwrap(),
            None
        );
        assert_eq!(
            parse_release(&release("2.0.0", false, true), v.clone()).unwrap(),
            None
        );
        assert!(parse_release("not json", v.clone()).is_err());
        assert!(parse_release(&release("not-a-version", false, false), v).is_err());
    }

    #[tokio::test]
    async fn http_failures_are_suppressed_at_the_boundary() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(10))
            .build()
            .unwrap();
        assert!(check_latest_at(&client, "http://[malformed").await.is_err());
    }
}
