//! Secret-safe, local-only Adapt conversation snapshots.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Completed,
}

impl SessionStatus {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Active => "interrupted",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub timestamp_ms: u128,
    pub kind: SessionEntryKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntryKind {
    Prompt {
        text: String,
    },
    Response {
        content: Vec<Value>,
        structured_result: Option<Value>,
        citations: Vec<Value>,
        remote_chat_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub started_at_ms: u128,
    pub updated_at_ms: u128,
    pub status: SessionStatus,
    pub remote_chat_id: Option<String>,
    #[serde(default)]
    pub resumed_from_session_id: Option<String>,
    pub entries: Vec<SessionEntry>,
}

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("could not create local session history at {path}")]
    CreateDirectory { path: PathBuf },
    #[error("could not write local session history at {path}")]
    Write { path: PathBuf },
    #[error("could not read local session history at {path}")]
    Read { path: PathBuf },
    #[error("local session `{id}` was not found")]
    NotFound { id: String },
    #[error("local session history at {path} is malformed")]
    Malformed { path: PathBuf },
}

#[derive(Clone)]
pub struct SessionHistory {
    directory: PathBuf,
    token: String,
}

impl SessionHistory {
    pub fn for_credential_file(
        credential_file: impl AsRef<Path>,
        token: impl Into<String>,
    ) -> Self {
        let directory = credential_file
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("sessions");
        Self {
            directory,
            token: token.into(),
        }
    }

    pub fn at(directory: impl Into<PathBuf>, token: impl Into<String>) -> Self {
        Self {
            directory: directory.into(),
            token: token.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn create(&self) -> Result<Session, HistoryError> {
        self.create_continuation(None, None)
    }

    pub fn create_continuation(
        &self,
        resumed_from_session_id: Option<&str>,
        remote_chat_id: Option<&str>,
    ) -> Result<Session, HistoryError> {
        let now = timestamp_ms();
        let mut session = Session {
            id: format!(
                "{}-{}-{}",
                now,
                std::process::id(),
                SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            started_at_ms: now,
            updated_at_ms: now,
            status: SessionStatus::Active,
            remote_chat_id: remote_chat_id.map(|id| self.redact_text(id)),
            resumed_from_session_id: resumed_from_session_id.map(str::to_owned),
            entries: Vec::new(),
        };
        self.save(&mut session)?;
        Ok(session)
    }

    pub fn append_prompt(&self, session: &mut Session, prompt: &str) -> Result<(), HistoryError> {
        session.entries.push(SessionEntry {
            timestamp_ms: timestamp_ms(),
            kind: SessionEntryKind::Prompt {
                text: self.redact_text(prompt),
            },
        });
        self.save(session)
    }

    pub fn append_response(
        &self,
        session: &mut Session,
        content: Vec<Value>,
        structured_result: Option<Value>,
        remote_chat_id: Option<String>,
    ) -> Result<(), HistoryError> {
        let content = content
            .into_iter()
            .map(|value| self.redact_value(value))
            .collect::<Vec<_>>();
        let structured_result = structured_result.map(|value| self.redact_value(value));
        let citations = content.iter().flat_map(find_citations).collect();
        let remote_chat_id = remote_chat_id.map(|id| self.redact_text(&id));
        if let Some(chat_id) = remote_chat_id.clone() {
            session.remote_chat_id = Some(chat_id);
        }
        session.entries.push(SessionEntry {
            timestamp_ms: timestamp_ms(),
            kind: SessionEntryKind::Response {
                content,
                structured_result,
                citations,
                remote_chat_id,
            },
        });
        self.save(session)
    }

    pub fn complete(&self, session: &mut Session) -> Result<(), HistoryError> {
        session.status = SessionStatus::Completed;
        self.save(session)
    }

    pub fn list(&self) -> Result<Vec<Session>, HistoryError> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.directory).map_err(|_| HistoryError::Read {
            path: self.directory.clone(),
        })? {
            let path = entry
                .map_err(|_| HistoryError::Read {
                    path: self.directory.clone(),
                })?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                sessions.push(load_file(&path)?);
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
        Ok(sessions)
    }

    pub fn load(&self, id: &str) -> Result<Session, HistoryError> {
        load_file(&self.path_for(id)).map_err(|error| match error {
            HistoryError::Read { .. } => HistoryError::NotFound { id: id.to_owned() },
            other => other,
        })
    }

    fn save(&self, session: &mut Session) -> Result<(), HistoryError> {
        fs::create_dir_all(&self.directory).map_err(|_| HistoryError::CreateDirectory {
            path: self.directory.clone(),
        })?;
        session.updated_at_ms = timestamp_ms();
        let path = self.path_for(&session.id);
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let serialized = serde_json::to_vec_pretty(session)
            .map_err(|_| HistoryError::Write { path: path.clone() })?;
        fs::write(&temporary, serialized)
            .map_err(|_| HistoryError::Write { path: path.clone() })?;
        fs::rename(&temporary, &path).map_err(|_| HistoryError::Write { path })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.directory.join(format!("{id}.json"))
    }

    fn redact_text(&self, text: &str) -> String {
        let without_token = if self.token.is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.token, "[redacted credential]")
        };
        redact_bearer(&without_token)
    }

    fn redact_value(&self, value: Value) -> Value {
        match value {
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let sensitive = [
                            "token",
                            "authorization",
                            "credential",
                            "secret",
                            "password",
                            "api_key",
                        ]
                        .iter()
                        .any(|term| key.to_ascii_lowercase().contains(term));
                        (
                            key,
                            if sensitive {
                                Value::String("[redacted]".into())
                            } else {
                                self.redact_value(value)
                            },
                        )
                    })
                    .collect(),
            ),
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| self.redact_value(value))
                    .collect(),
            ),
            Value::String(text) => Value::String(self.redact_text(&text)),
            other => other,
        }
    }
}

fn load_file(path: &Path) -> Result<Session, HistoryError> {
    let text = fs::read_to_string(path).map_err(|_| HistoryError::Read {
        path: path.to_path_buf(),
    })?;
    serde_json::from_str(&text).map_err(|_| HistoryError::Malformed {
        path: path.to_path_buf(),
    })
}

fn find_citations(value: &Value) -> Vec<Value> {
    match value {
        Value::Object(map) => map
            .iter()
            .flat_map(|(key, value)| {
                let mut found = if key.to_ascii_lowercase().contains("citation") {
                    vec![value.clone()]
                } else {
                    Vec::new()
                };
                found.extend(find_citations(value));
                found
            })
            .collect(),
        Value::Array(values) => values.iter().flat_map(find_citations).collect(),
        _ => Vec::new(),
    }
}

fn redact_bearer(text: &str) -> String {
    let mut words = text.split_whitespace();
    let mut output = String::new();
    while let Some(word) = words.next() {
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(word);
        if word.eq_ignore_ascii_case("bearer") && words.next().is_some() {
            output.push_str(" [redacted]");
        }
    }
    output
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn history(name: &str) -> (SessionHistory, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("adapt-tui-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        (
            SessionHistory::at(&directory, "very-secret-token"),
            directory,
        )
    }
    #[test]
    fn snapshots_round_trip_and_active_sessions_are_recoverable() {
        let (history, directory) = history("round-trip");
        let mut session = history.create().unwrap();
        history
            .append_prompt(&mut session, "Bearer very-secret-token show citation")
            .unwrap();
        history
            .append_response(
                &mut session,
                vec![serde_json::json!({"text":"ok", "citation":"guide", "authorization":"nope"})],
                Some(serde_json::json!({"answer": 1, "secret": "nope"})),
                Some("chat-1".into()),
            )
            .unwrap();
        let loaded = history.load(&session.id).unwrap();
        assert_eq!(loaded.status, SessionStatus::Active);
        assert_eq!(loaded.remote_chat_id.as_deref(), Some("chat-1"));
        assert_eq!(loaded.entries.len(), 2);
        assert!(loaded.entries[1].timestamp_ms > 0);
        let file = fs::read_to_string(history.path_for(&session.id)).unwrap();
        assert!(!file.contains("very-secret-token"));
        assert!(!file.contains("Bearer very-secret-token"));
        assert!(!file.contains("\"nope\""));
        assert!(file.contains("citation"));
        history.complete(&mut session).unwrap();
        assert_eq!(
            history.load(&session.id).unwrap().status,
            SessionStatus::Completed
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn continuation_sessions_retain_their_origin_and_remote_chat_id() {
        let (history, directory) = history("continuation");
        let session = history
            .create_continuation(Some("previous-session"), Some("chat-123"))
            .unwrap();
        let loaded = history.load(&session.id).unwrap();
        assert_eq!(
            loaded.resumed_from_session_id.as_deref(),
            Some("previous-session")
        );
        assert_eq!(loaded.remote_chat_id.as_deref(), Some("chat-123"));
        let _ = fs::remove_dir_all(directory);
    }
    #[test]
    fn uses_a_sessions_sibling_not_the_credential_file() {
        let history = SessionHistory::for_credential_file("/tmp/.adapt/config.toml", "token");
        assert_eq!(history.directory(), Path::new("/tmp/.adapt/sessions"));
    }
}
