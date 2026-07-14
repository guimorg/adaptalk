//! Secret-safe, local-only Adapt conversation snapshots.
//!
//! This module owns file lifecycle and accepts only redacted transcript data.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn generate() -> Self {
        Self(format!(
            "{}-{}-{}",
            timestamp_ms(),
            std::process::id(),
            SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    pub fn parse(value: &str) -> Result<Self, HistoryError> {
        let valid = value.split('-').count() == 3
            && value
                .split('-')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(HistoryError::InvalidId {
                id: value.to_owned(),
            })
        }
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

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
pub struct TextBlock {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptResponse {
    pub text_blocks: Vec<TextBlock>,
    /// A deliberately opaque, already-redacted result for inline display.
    pub structured_result: Option<serde_json::Value>,
    pub remote_chat_id: Option<String>,
}

/// Data that has passed through the application redaction boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactedText(String);

impl RedactedText {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }
}

/// A display response that has passed through the application redaction boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactedTranscriptResponse(TranscriptResponse);

impl RedactedTranscriptResponse {
    pub(crate) fn new(value: TranscriptResponse) -> Self {
        Self(value)
    }

    pub fn as_inner(&self) -> &TranscriptResponse {
        &self.0
    }

    pub fn into_inner(self) -> TranscriptResponse {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntryKind {
    Prompt { text: String },
    Response(TranscriptResponse),
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub started_at_ms: u128,
    pub updated_at_ms: u128,
    pub status: SessionStatus,
    #[serde(default)]
    pub resumed_from_session_id: Option<SessionId>,
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
    #[error("local session ID `{id}` is invalid")]
    InvalidId { id: String },
    #[error("local session history at {path} is malformed")]
    Malformed { path: PathBuf },
}

#[derive(Clone)]
pub struct SessionHistory {
    directory: PathBuf,
}

impl SessionHistory {
    pub fn for_credential_file(credential_file: impl AsRef<Path>) -> Self {
        let directory = credential_file
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("sessions");
        Self { directory }
    }
    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn create(&self) -> Result<Session, HistoryError> {
        self.create_continuation(None)
    }
    pub fn create_continuation(
        &self,
        resumed_from_session_id: Option<SessionId>,
    ) -> Result<Session, HistoryError> {
        let now = timestamp_ms();
        let mut session = Session {
            id: SessionId::generate(),
            started_at_ms: now,
            updated_at_ms: now,
            status: SessionStatus::Active,
            resumed_from_session_id,
            entries: vec![],
        };
        self.save(&mut session)?;
        Ok(session)
    }
    pub fn append_prompt(
        &self,
        session: &mut Session,
        text: RedactedText,
    ) -> Result<(), HistoryError> {
        self.append(session, SessionEntryKind::Prompt { text: text.0 })
    }
    pub fn append_response(
        &self,
        session: &mut Session,
        response: RedactedTranscriptResponse,
    ) -> Result<(), HistoryError> {
        self.append(session, SessionEntryKind::Response(response.0))
    }
    pub fn append_error(
        &self,
        session: &mut Session,
        message: RedactedText,
    ) -> Result<(), HistoryError> {
        self.append(session, SessionEntryKind::Error { message: message.0 })
    }
    pub fn complete(&self, session: &mut Session) -> Result<(), HistoryError> {
        session.status = SessionStatus::Completed;
        self.save(session)
    }
    pub fn list(&self) -> Result<Vec<Session>, HistoryError> {
        if !self.directory.exists() {
            return Ok(vec![]);
        }
        let mut sessions = fs::read_dir(&self.directory)
            .map_err(|_| HistoryError::Read {
                path: self.directory.clone(),
            })?
            .map(|entry| {
                entry.map_err(|_| HistoryError::Read {
                    path: self.directory.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .map(|path| load_file(&path))
            .collect::<Result<Vec<_>, _>>()?;
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
        Ok(sessions)
    }
    pub fn load(&self, id: &str) -> Result<Session, HistoryError> {
        let id = SessionId::parse(id)?;
        load_file(&self.path_for(&id)).map_err(|error| match error {
            HistoryError::Read { .. } => HistoryError::NotFound { id: id.to_string() },
            other => other,
        })
    }
    /// Resolve continuation from this session's responses, then its immutable lineage.
    pub fn latest_remote_chat_id(&self, session: &Session) -> Result<Option<String>, HistoryError> {
        let mut current = session.clone();
        loop {
            if let Some(chat_id) = current.latest_remote_chat_id() {
                return Ok(Some(chat_id.to_owned()));
            }
            let Some(origin) = current.resumed_from_session_id else {
                return Ok(None);
            };
            current = self.load(&origin.to_string())?;
        }
    }
    fn append(&self, session: &mut Session, kind: SessionEntryKind) -> Result<(), HistoryError> {
        session.entries.push(SessionEntry {
            timestamp_ms: timestamp_ms(),
            kind,
        });
        self.save(session)
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
    fn path_for(&self, id: &SessionId) -> PathBuf {
        self.directory.join(format!("{id}.json"))
    }
}

impl Session {
    /// The latest response is the single source of truth for remote continuation.
    pub fn latest_remote_chat_id(&self) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find_map(|entry| match &entry.kind {
                SessionEntryKind::Response(response) => response.remote_chat_id.as_deref(),
                _ => None,
            })
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
        (SessionHistory::at(&directory), directory)
    }
    #[test]
    fn snapshots_round_trip_and_active_sessions_are_recoverable() {
        let (history, directory) = history("round-trip");
        let mut session = history.create().unwrap();
        history
            .append_prompt(&mut session, RedactedText::new("prompt".into()))
            .unwrap();
        history
            .append_response(
                &mut session,
                RedactedTranscriptResponse::new(TranscriptResponse {
                    text_blocks: vec![TextBlock { text: "ok".into() }],
                    structured_result: Some(serde_json::json!({"answer": 1})),
                    remote_chat_id: Some("chat-1".into()),
                }),
            )
            .unwrap();
        history
            .append_error(&mut session, RedactedText::new("failed".into()))
            .unwrap();
        let loaded = history.load(&session.id.to_string()).unwrap();
        assert_eq!(loaded.status, SessionStatus::Active);
        assert_eq!(loaded.entries.len(), 3);
        assert_eq!(loaded.latest_remote_chat_id(), Some("chat-1"));
        history.complete(&mut session).unwrap();
        assert_eq!(
            history.load(&session.id.to_string()).unwrap().status,
            SessionStatus::Completed
        );
        let _ = fs::remove_dir_all(directory);
    }
    #[test]
    fn invalid_ids_cannot_escape_the_sessions_directory() {
        let (history, directory) = history("invalid-id");
        for id in ["../secret", "/tmp/other", "valid/other", "not-a-session"] {
            assert!(matches!(
                history.load(id),
                Err(HistoryError::InvalidId { .. })
            ));
        }
        let _ = fs::remove_dir_all(directory);
    }
    #[test]
    fn continuation_sessions_retain_their_origin() {
        let (history, directory) = history("continuation");
        let origin = SessionId::parse("1-2-3").unwrap();
        let session = history.create_continuation(Some(origin.clone())).unwrap();
        let loaded = history.load(&session.id.to_string()).unwrap();
        assert_eq!(loaded.resumed_from_session_id, Some(origin));
        let _ = fs::remove_dir_all(directory);
    }
    #[test]
    fn continuation_uses_the_latest_response_from_its_origin() {
        let (history, directory) = history("continuation-chat-id");
        let mut origin = history.create().unwrap();
        history
            .append_response(
                &mut origin,
                RedactedTranscriptResponse::new(TranscriptResponse {
                    text_blocks: vec![],
                    structured_result: None,
                    remote_chat_id: Some("chat-1".into()),
                }),
            )
            .unwrap();
        let continuation = history.create_continuation(Some(origin.id)).unwrap();
        assert_eq!(
            history.latest_remote_chat_id(&continuation).unwrap(),
            Some("chat-1".into())
        );
        let _ = fs::remove_dir_all(directory);
    }
    #[test]
    fn uses_a_sessions_sibling_not_the_credential_file() {
        assert_eq!(
            SessionHistory::for_credential_file("/tmp/.adapt/config.toml").directory(),
            Path::new("/tmp/.adapt/sessions")
        );
    }
}
