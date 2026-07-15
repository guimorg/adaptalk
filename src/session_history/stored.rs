//! Private serde representation of local session snapshots.

use serde::{Deserialize, Serialize};

use super::{Session, SessionEntry, SessionEntryKind, SessionId, SessionStatus};
use crate::transcript::{TextBlock, TranscriptResponse};

/// The on-disk representation is private so SessionHistory remains the only
/// serialization seam for redacted sessions.
#[derive(Serialize, Deserialize)]
pub(super) struct StoredSession {
    id: String,
    started_at_ms: u128,
    updated_at_ms: u128,
    status: StoredSessionStatus,
    #[serde(default)]
    resumed_from_session_id: Option<String>,
    entries: Vec<StoredSessionEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSessionStatus {
    Active,
    Completed,
}

#[derive(Serialize, Deserialize)]
struct StoredSessionEntry {
    timestamp_ms: u128,
    kind: StoredSessionEntryKind,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredSessionEntryKind {
    Prompt { text: String },
    Response(StoredTranscriptResponse),
    Error { message: String },
}

#[derive(Serialize, Deserialize)]
struct StoredTranscriptResponse {
    text_blocks: Vec<StoredTextBlock>,
    structured_result: Option<serde_json::Value>,
    remote_chat_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredTextBlock {
    text: String,
}

impl From<&Session> for StoredSession {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.to_string(),
            started_at_ms: session.started_at_ms,
            updated_at_ms: session.updated_at_ms,
            status: match session.status {
                SessionStatus::Active => StoredSessionStatus::Active,
                SessionStatus::Completed => StoredSessionStatus::Completed,
            },
            resumed_from_session_id: session
                .resumed_from_session_id
                .as_ref()
                .map(ToString::to_string),
            entries: session
                .entries
                .iter()
                .map(StoredSessionEntry::from)
                .collect(),
        }
    }
}

impl TryFrom<StoredSession> for Session {
    type Error = ();

    fn try_from(session: StoredSession) -> Result<Self, Self::Error> {
        Ok(Self {
            id: SessionId::parse(&session.id).map_err(|_| ())?,
            started_at_ms: session.started_at_ms,
            updated_at_ms: session.updated_at_ms,
            status: match session.status {
                StoredSessionStatus::Active => SessionStatus::Active,
                StoredSessionStatus::Completed => SessionStatus::Completed,
            },
            resumed_from_session_id: session
                .resumed_from_session_id
                .map(|id| SessionId::parse(&id))
                .transpose()
                .map_err(|_| ())?,
            entries: session
                .entries
                .into_iter()
                .map(SessionEntry::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl From<&SessionEntry> for StoredSessionEntry {
    fn from(entry: &SessionEntry) -> Self {
        Self {
            timestamp_ms: entry.timestamp_ms,
            kind: match &entry.kind {
                SessionEntryKind::Prompt { text } => {
                    StoredSessionEntryKind::Prompt { text: text.clone() }
                }
                SessionEntryKind::Response(response) => {
                    StoredSessionEntryKind::Response(StoredTranscriptResponse::from(response))
                }
                SessionEntryKind::Error { message } => StoredSessionEntryKind::Error {
                    message: message.clone(),
                },
            },
        }
    }
}

impl TryFrom<StoredSessionEntry> for SessionEntry {
    type Error = ();

    fn try_from(entry: StoredSessionEntry) -> Result<Self, Self::Error> {
        Ok(Self {
            timestamp_ms: entry.timestamp_ms,
            kind: match entry.kind {
                StoredSessionEntryKind::Prompt { text } => SessionEntryKind::Prompt { text },
                StoredSessionEntryKind::Response(response) => {
                    SessionEntryKind::Response(TranscriptResponse::from(response))
                }
                StoredSessionEntryKind::Error { message } => SessionEntryKind::Error { message },
            },
        })
    }
}

impl From<&TranscriptResponse> for StoredTranscriptResponse {
    fn from(response: &TranscriptResponse) -> Self {
        Self {
            text_blocks: response
                .text_blocks
                .iter()
                .map(|block| StoredTextBlock {
                    text: block.text.clone(),
                })
                .collect(),
            structured_result: response.structured_result.clone(),
            remote_chat_id: response.remote_chat_id.clone(),
        }
    }
}

impl From<StoredTranscriptResponse> for TranscriptResponse {
    fn from(response: StoredTranscriptResponse) -> Self {
        Self {
            text_blocks: response
                .text_blocks
                .into_iter()
                .map(|block| TextBlock { text: block.text })
                .collect(),
            structured_result: response.structured_result,
            remote_chat_id: response.remote_chat_id,
        }
    }
}
