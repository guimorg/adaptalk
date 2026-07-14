//! State transitions for local Adapt conversations, independent of connection setup.

use std::{future::Future, pin::Pin};

use anyhow::Result;

use crate::{
    redaction::Redactor,
    session_history::{RedactedTranscriptResponse, Session, SessionHistory, TranscriptResponse},
};

pub type QueryFuture<'a> = Pin<Box<dyn Future<Output = Result<RedactedTranscriptResponse>> + 'a>>;

/// The narrow boundary needed to submit a prompt to an already-connected service.
pub trait ConversationQuery {
    fn query<'a>(&'a self, prompt: &'a str, continuation: Option<&'a str>) -> QueryFuture<'a>;
}

pub struct Connection<Q> {
    pub query: Q,
    pub history: SessionHistory,
    pub redactor: Redactor,
}

pub enum RenderIntent {
    ShowHistory(Session),
}

enum ConversationState<Q> {
    Disconnected,
    ViewingHistory(Session),
    Connected(ActiveConversation<Q>),
}

struct ActiveConversation<Q> {
    query: Q,
    history: SessionHistory,
    session: Session,
}

pub struct ConversationController<Q> {
    state: ConversationState<Q>,
    offline_history: SessionHistory,
    redactor: Redactor,
}

impl<Q: ConversationQuery> ConversationController<Q> {
    pub fn new(offline_history: SessionHistory) -> Self {
        Self {
            state: ConversationState::Disconnected,
            offline_history,
            redactor: Redactor::default(),
        }
    }

    pub fn history(&self) -> &SessionHistory {
        &self.offline_history
    }

    pub fn redactor(&self) -> Redactor {
        self.redactor.clone()
    }

    pub fn needs_connection(&self) -> bool {
        !matches!(self.state, ConversationState::Connected(_))
    }

    pub fn viewing_session(&self) -> Option<&Session> {
        match &self.state {
            ConversationState::ViewingHistory(session) => Some(session),
            _ => None,
        }
    }

    /// Resolve external connection work before applying the local transition.
    /// A failed future leaves the current state untouched.
    pub async fn connect_with<F>(&mut self, connection: F) -> Result<()>
    where
        F: Future<Output = Result<Connection<Q>>>,
    {
        self.connect(connection.await?)
    }

    /// Start a fresh local session, or a continuation of the viewed session.
    /// Session creation succeeds before state changes, so an error leaves the view intact.
    pub fn connect(&mut self, connection: Connection<Q>) -> Result<()> {
        if matches!(self.state, ConversationState::Connected(_)) {
            return Ok(());
        }
        let resumed_from = match &self.state {
            ConversationState::ViewingHistory(session) => Some(session.id.clone()),
            ConversationState::Disconnected => None,
            ConversationState::Connected(_) => unreachable!(),
        };
        let session = if let Some(origin) = resumed_from {
            connection.history.create_continuation(Some(origin))?
        } else {
            connection.history.create()?
        };
        self.redactor = connection.redactor;
        self.state = ConversationState::Connected(ActiveConversation {
            query: connection.query,
            history: connection.history,
            session,
        });
        Ok(())
    }

    pub fn open(&mut self, session: Session) -> Result<RenderIntent> {
        self.finish()?;
        self.state = ConversationState::ViewingHistory(session.clone());
        Ok(RenderIntent::ShowHistory(session))
    }

    pub fn finish(&mut self) -> Result<()> {
        if let ConversationState::Connected(active) = &mut self.state {
            active.history.complete(&mut active.session)?;
        }
        Ok(())
    }

    pub async fn submit(&mut self, prompt: &str) -> Result<TranscriptResponse> {
        let ConversationState::Connected(active) = &mut self.state else {
            anyhow::bail!("a connection is required before submitting a prompt");
        };
        active
            .history
            .append_prompt(&mut active.session, self.redactor.transcript_text(prompt))?;
        let continuation = active.session.latest_remote_chat_id();
        match active.query.query(prompt, continuation).await {
            Ok(response) => {
                let display = response.as_inner().clone();
                active
                    .history
                    .append_response(&mut active.session, response)?;
                Ok(display)
            }
            Err(error) => {
                active.history.append_error(
                    &mut active.session,
                    self.redactor.transcript_text(&error.to_string()),
                )?;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::session_history::{TextBlock, TranscriptResponse};

    #[derive(Clone)]
    struct Query {
        calls: Rc<RefCell<Vec<Option<String>>>>,
        results: Rc<RefCell<Vec<Result<RedactedTranscriptResponse>>>>,
    }

    impl ConversationQuery for Query {
        fn query<'a>(&'a self, _prompt: &'a str, continuation: Option<&'a str>) -> QueryFuture<'a> {
            self.calls
                .borrow_mut()
                .push(continuation.map(str::to_owned));
            Box::pin(async move { self.results.borrow_mut().remove(0) })
        }
    }

    fn response(redactor: &Redactor, chat_id: Option<&str>) -> RedactedTranscriptResponse {
        redactor.transcript_response(TranscriptResponse {
            text_blocks: vec![TextBlock { text: "ok".into() }],
            structured_result: None,
            remote_chat_id: chat_id.map(str::to_owned),
        })
    }

    fn history(name: &str) -> (SessionHistory, std::path::PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "adapt-tui-controller-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        (SessionHistory::at(&directory), directory)
    }

    #[tokio::test]
    async fn opening_history_then_connecting_creates_a_local_continuation() {
        let (history, directory) = history("open");
        let origin = history.create().unwrap();
        let mut controller = ConversationController::new(history.clone());
        controller.open(origin.clone()).unwrap();
        let redactor = Redactor::default();
        let query = Query {
            calls: Rc::new(RefCell::new(vec![])),
            results: Rc::new(RefCell::new(vec![Ok(response(&redactor, None))])),
        };
        controller
            .connect(Connection {
                query,
                history: history.clone(),
                redactor,
            })
            .unwrap();
        controller.submit("next").await.unwrap();
        let sessions = history.list().unwrap();
        assert!(
            sessions
                .iter()
                .any(|session| session.resumed_from_session_id.as_ref() == Some(&origin.id))
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn failed_query_is_persisted_and_the_connection_remains_usable() {
        let (history, directory) = history("error");
        let mut controller = ConversationController::new(history.clone());
        let query = Query {
            calls: Rc::new(RefCell::new(vec![])),
            results: Rc::new(RefCell::new(vec![Err(anyhow::anyhow!("offline"))])),
        };
        controller
            .connect(Connection {
                query,
                history: history.clone(),
                redactor: Redactor::default(),
            })
            .unwrap();
        assert!(controller.submit("prompt").await.is_err());
        assert!(!controller.needs_connection());
        assert!(
            history
                .list()
                .unwrap()
                .iter()
                .any(|session| session.entries.iter().any(|entry| matches!(
                    entry.kind,
                    crate::session_history::SessionEntryKind::Error { .. }
                )))
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn failed_connection_restores_the_history_view() {
        let (history, directory) = history("failed-connect");
        let opened = history.create().unwrap();
        let mut controller: ConversationController<Query> = ConversationController::new(history);
        controller.open(opened.clone()).unwrap();
        assert!(
            controller
                .connect_with(async { Err(anyhow::anyhow!("offline")) })
                .await
                .is_err()
        );
        assert_eq!(
            controller.viewing_session().map(|session| &session.id),
            Some(&opened.id)
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn continuation_is_derived_from_the_latest_transcript_response() {
        let (history, directory) = history("continuation");
        let redactor = Redactor::default();
        let calls = Rc::new(RefCell::new(vec![]));
        let query = Query {
            calls: calls.clone(),
            results: Rc::new(RefCell::new(vec![
                Ok(response(&redactor, Some("chat-1"))),
                Ok(response(&redactor, Some("chat-2"))),
            ])),
        };
        let mut controller = ConversationController::new(history.clone());
        controller
            .connect(Connection {
                query,
                history,
                redactor,
            })
            .unwrap();
        controller.submit("first").await.unwrap();
        controller.submit("second").await.unwrap();
        assert_eq!(*calls.borrow(), vec![None, Some("chat-1".into())]);
        let _ = std::fs::remove_dir_all(directory);
    }
}
