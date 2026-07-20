//! State transitions for local Adapt conversations, independent of connection setup.

use std::pin::Pin;

use anyhow::Result;

use crate::{
    redaction::Redactor,
    session_history::{Session, SessionHistory},
    transcript::TranscriptResponse,
};

pub type QueryFuture<'a> = Pin<Box<dyn Future<Output = Result<TranscriptResponse>> + 'a>>;

/// The narrow boundary needed to submit a prompt to an already-connected service.
pub trait ConversationQuery {
    fn query<'a>(&'a self, prompt: &'a str, continuation: Option<&'a str>) -> QueryFuture<'a>;
}

pub struct Connection<Q> {
    pub query: Q,
    pub redactor: Redactor,
}

pub enum SubmitOutcome {
    Response(TranscriptResponse),
    ResponseWithPersistenceWarning {
        response: TranscriptResponse,
        error: crate::session_history::HistoryError,
    },
    ErrorWithPersistenceWarning {
        error: anyhow::Error,
        persistence_error: crate::session_history::HistoryError,
    },
    /// The remote continuation attempt failed; the continuation has been dropped and subsequent
    /// prompts will start a fresh Adapt session.
    ContinuationFailed {
        error: anyhow::Error,
    },
    ContinuationFailedWithPersistenceWarning {
        error: anyhow::Error,
        persistence_error: crate::session_history::HistoryError,
    },
}

enum ConversationState<Q> {
    Disconnected,
    ViewingHistory(Session),
    Connected(ActiveConversation<Q>),
}

struct ActiveConversation<Q> {
    query: Q,
    session: Session,
    pending_response: Option<crate::session_history::RedactedTranscriptResponse>,
    continuation_exhausted: bool,
}

pub struct ConversationController<Q> {
    state: ConversationState<Q>,
    history: SessionHistory,
    redactor: Redactor,
}

impl<Q: ConversationQuery> ConversationController<Q> {
    pub fn new(history: SessionHistory) -> Self {
        Self {
            state: ConversationState::Disconnected,
            history,
            redactor: Redactor::default(),
        }
    }

    pub fn history(&self) -> &SessionHistory {
        &self.history
    }

    /// Redact terminal-only values that arise outside a submitted transcript.
    pub fn redact(&self, value: &str) -> String {
        self.redactor.text(value)
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

    pub fn viewing_continuation(&self) -> Result<Option<String>> {
        let Some(session) = self.viewing_session() else {
            anyhow::bail!("no session is open for viewing");
        };
        Ok(self.history.latest_remote_chat_id(session)?)
    }

    /// Start a fresh local session, or a continuation of the viewed session.
    /// Session creation succeeds before state changes, so an error leaves the view intact.
    pub fn connect(&mut self, connection: Connection<Q>) -> Result<()> {
        if matches!(self.state, ConversationState::Connected(_)) {
            return Ok(());
        }
        let resumed_from = match &self.state {
            ConversationState::ViewingHistory(session) => Some(session.id().clone()),
            ConversationState::Disconnected => None,
            ConversationState::Connected(_) => return Ok(()),
        };
        let session = if let Some(origin) = resumed_from {
            self.history.create_continuation(Some(origin))?
        } else {
            self.history.create()?
        };
        self.redactor = connection.redactor;
        self.state = ConversationState::Connected(ActiveConversation {
            query: connection.query,
            session,
            pending_response: None,
            continuation_exhausted: false,
        });
        Ok(())
    }

    pub fn open(&mut self, session: Session) -> Result<Session> {
        self.finish()?;
        self.state = ConversationState::ViewingHistory(session.clone());
        Ok(session)
    }

    pub fn finish(&mut self) -> Result<()> {
        if let ConversationState::Connected(active) = &mut self.state {
            if active.pending_response.is_some() {
                anyhow::bail!(
                    "the last remote response is not persisted; retry persistence or abandon it before finishing"
                );
            }
            self.history.complete(&mut active.session)?;
            self.state = ConversationState::Disconnected;
        }
        Ok(())
    }

    /// Retry the one remote response whose local persistence failed.
    pub fn retry_pending_response(&mut self) -> Result<bool> {
        let ConversationState::Connected(active) = &mut self.state else {
            anyhow::bail!("a connection is required before retrying persistence");
        };
        let Some(response) = active.pending_response.take() else {
            return Ok(false);
        };
        match self
            .history
            .append_response(&mut active.session, response.clone())
        {
            Ok(()) => Ok(true),
            Err(error) => {
                active.pending_response = Some(response);
                Err(error.into())
            }
        }
    }

    /// Explicitly discard an unpersisted response before abandoning the local session.
    pub fn abandon_pending_response(&mut self) -> Result<bool> {
        let ConversationState::Connected(active) = &mut self.state else {
            anyhow::bail!("a connection is required before abandoning persistence");
        };
        Ok(active.pending_response.take().is_some())
    }

    /// Persist the original user input while submitting its resolved outbound form.
    pub async fn submit(&mut self, prompt: PromptSubmission) -> Result<SubmitOutcome> {
        let ConversationState::Connected(active) = &mut self.state else {
            anyhow::bail!("a connection is required before submitting a prompt");
        };
        if active.pending_response.is_some() {
            anyhow::bail!(
                "the previous remote response could not be saved; retry persistence or abandon this session before submitting again"
            );
        }
        self.history.append_prompt(
            &mut active.session,
            self.redactor.transcript_text(&prompt.original),
        )?;
        let continuation = if active.continuation_exhausted {
            None
        } else {
            self.history.latest_remote_chat_id(&active.session)?
        };
        let used_continuation = continuation.is_some();
        match active
            .query
            .query(prompt.outbound(), continuation.as_deref())
            .await
        {
            Ok(response) => {
                let response = self.redactor.transcript_response(response);
                let display = response.as_inner().clone();
                match self
                    .history
                    .append_response(&mut active.session, response.clone())
                {
                    Ok(()) => Ok(SubmitOutcome::Response(display)),
                    Err(error) => {
                        active.pending_response = Some(response);
                        Ok(SubmitOutcome::ResponseWithPersistenceWarning {
                            response: display,
                            error,
                        })
                    }
                }
            }
            Err(error) => {
                if used_continuation {
                    active.continuation_exhausted = true;
                }
                match self.history.append_error(
                    &mut active.session,
                    self.redactor.transcript_text(&error.to_string()),
                ) {
                    Ok(()) => {
                        if used_continuation {
                            Ok(SubmitOutcome::ContinuationFailed { error })
                        } else {
                            Err(error)
                        }
                    }
                    Err(persistence_error) => {
                        if used_continuation {
                            Ok(SubmitOutcome::ContinuationFailedWithPersistenceWarning {
                                error,
                                persistence_error,
                            })
                        } else {
                            Ok(SubmitOutcome::ErrorWithPersistenceWarning {
                                error,
                                persistence_error,
                            })
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::transcript::{TextBlock, TranscriptResponse};

    #[derive(Clone)]
    struct Query {
        calls: Rc<RefCell<Vec<Option<String>>>>,
        results: Rc<RefCell<Vec<Result<TranscriptResponse>>>>,
    }

    impl ConversationQuery for Query {
        fn query<'a>(&'a self, _prompt: &'a str, continuation: Option<&'a str>) -> QueryFuture<'a> {
            self.calls
                .borrow_mut()
                .push(continuation.map(str::to_owned));
            Box::pin(async move { self.results.borrow_mut().remove(0) })
        }
    }

    fn response(chat_id: Option<&str>) -> TranscriptResponse {
        TranscriptResponse {
            text_blocks: vec![TextBlock { text: "ok".into() }],
            structured_result: None,
            remote_chat_id: chat_id.map(str::to_owned),
        }
    }

    fn history(name: &str) -> (SessionHistory, std::path::PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("adaptalk-controller-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        (SessionHistory::at(&directory), directory)
    }

    #[tokio::test]
    async fn continuation_failure_drops_the_continuation_for_subsequent_prompts() {
        let (history, directory) = history("continuation-failure");
        let mut origin = history.create().unwrap();
        let origin_response = Redactor::default().transcript_response(response(Some("chat-1")));
        history
            .append_response(&mut origin, origin_response)
            .unwrap();
        let mut controller = ConversationController::new(history.clone());
        controller.open(origin.clone()).unwrap();
        let calls = Rc::new(RefCell::new(vec![]));
        let query = Query {
            calls: calls.clone(),
            results: Rc::new(RefCell::new(vec![
                Err(anyhow::anyhow!("session expired")),
                Ok(response(None)),
            ])),
        };
        controller
            .connect(Connection {
                query,
                redactor: Redactor::default(),
            })
            .unwrap();
        assert!(matches!(
            controller.submit(submission("first")).await.unwrap(),
    fn submission(prompt: &str) -> PromptSubmission {
        PromptSubmission::unchanged(prompt)
    }

            SubmitOutcome::ContinuationFailed { .. }
        ));
        assert!(matches!(
            controller.submit(submission("second")).await.unwrap(),
            SubmitOutcome::Response(_)
        ));
        assert_eq!(*calls.borrow(), vec![Some("chat-1".into()), None]);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn query_failure_without_continuation_is_a_plain_error() {
        let (history, directory) = history("plain-error");
        let mut controller = ConversationController::new(history.clone());
        let query = Query {
            calls: Rc::new(RefCell::new(vec![])),
            results: Rc::new(RefCell::new(vec![Err(anyhow::anyhow!("network error"))])),
        };
        controller
            .connect(Connection {
                query,
                redactor: Redactor::default(),
            })
            .unwrap();
        assert!(controller.submit(submission("prompt")).await.is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn opening_history_then_connecting_creates_a_local_continuation() {
        let (history, directory) = history("open");
        let mut origin = history.create().unwrap();
        let origin_response = Redactor::default().transcript_response(response(Some("chat-1")));
        history
            .append_response(&mut origin, origin_response)
            .unwrap();
        let mut controller = ConversationController::new(history.clone());
        controller.open(origin.clone()).unwrap();
        let redactor = Redactor::default();
        let calls = Rc::new(RefCell::new(vec![]));
        let query = Query {
            calls: calls.clone(),
            results: Rc::new(RefCell::new(vec![Ok(response(None)), Ok(response(None))])),
        };
        controller.connect(Connection { query, redactor }).unwrap();
        controller.submit(submission("next")).await.unwrap();
        controller
            .submit(submission("after missing chat ID"))
            .await
            .unwrap();
        let sessions = history.list().unwrap();
        assert!(
            sessions
                .iter()
                .any(|session| session.resumed_from_session_id() == Some(origin.id()))
        );
        assert_eq!(*calls.borrow(), vec![Some("chat-1".into()), None]);
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
                redactor: Redactor::default(),
            })
            .unwrap();
        assert!(controller.submit(submission("prompt")).await.is_err());
        assert!(!controller.needs_connection());
        assert!(
            history
                .list()
                .unwrap()
                .iter()
                .any(|session| session.entries().iter().any(|entry| matches!(
                    entry.kind(),
                    crate::session_history::SessionEntryKind::Error { .. }
                )))
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn opening_history_retains_the_history_view() {
        let (history, directory) = history("failed-connect");
        let opened = history.create().unwrap();
        let mut controller: ConversationController<Query> = ConversationController::new(history);
        assert_eq!(controller.open(opened.clone()).unwrap().id(), opened.id());
        assert_eq!(
            controller.viewing_session().map(Session::id),
            Some(opened.id())
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[derive(Clone)]
    struct SaveFailingQuery {
        directory: std::path::PathBuf,
        response: TranscriptResponse,
    }

    impl ConversationQuery for SaveFailingQuery {
        fn query<'a>(
            &'a self,
            _prompt: &'a str,
            _continuation: Option<&'a str>,
        ) -> QueryFuture<'a> {
            let directory = self.directory.clone();
            let response = self.response.clone();
            Box::pin(async move {
                std::fs::remove_dir_all(&directory).unwrap();
                std::fs::write(&directory, "not a directory").unwrap();
                Ok(response)
            })
        }
    }

    #[derive(Clone)]
    struct CapturingQuery {
        prompts: Rc<RefCell<Vec<String>>>,
    }

    impl ConversationQuery for CapturingQuery {
        fn query<'a>(&'a self, prompt: &'a str, _continuation: Option<&'a str>) -> QueryFuture<'a> {
            self.prompts.borrow_mut().push(prompt.into());
            Box::pin(async { Ok(response(None)) })
        }
    }

    #[tokio::test]
    async fn persists_original_prompt_while_submitting_outbound_prompt() {
        let (history, directory) = history("outbound-prompt");
        let prompts = Rc::new(RefCell::new(vec![]));
        let mut controller = ConversationController::new(history.clone());
        controller
            .connect(Connection {
                query: CapturingQuery {
                    prompts: prompts.clone(),
                },
                redactor: Redactor::default(),
            })
            .unwrap();

        controller
            .submit(PromptSubmission::expanded(
                "review @notes.md",
                "review <file path=\"@notes.md\">\nnotes\n</file>".into(),
            ))
            .await
            .unwrap();

        assert_eq!(
            *prompts.borrow(),
            vec!["review <file path=\"@notes.md\">\nnotes\n</file>"]
        );
        assert!(history.list().unwrap().iter().any(|session| {
            session.entries().iter().any(|entry| {
                matches!(
                    entry.kind(),
                    crate::session_history::SessionEntryKind::Prompt { text }
                        if text == "review @notes.md"
                )
            })
        }));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn remote_success_with_a_history_write_failure_is_not_an_error() {
        let (history, directory) = history("response-save-failure");
        let query = SaveFailingQuery {
            directory: directory.clone(),
            response: response(Some("chat-1")),
        };
        let mut controller = ConversationController::new(history.clone());
        controller
            .connect(Connection {
                query,
                redactor: Redactor::default(),
            })
            .unwrap();
        assert!(matches!(
            controller.submit(submission("prompt")).await.unwrap(),
            SubmitOutcome::ResponseWithPersistenceWarning { .. }
        ));
        assert!(
            controller
                .submit(submission("another prompt"))
                .await
                .is_err()
        );
        assert!(controller.finish().is_err());
        assert!(controller.abandon_pending_response().unwrap());
        std::fs::remove_file(directory).unwrap();
    }

    #[tokio::test]
    async fn continuation_is_derived_from_the_latest_transcript_response() {
        let (history, directory) = history("continuation");
        let redactor = Redactor::default();
        let calls = Rc::new(RefCell::new(vec![]));
        let query = Query {
            calls: calls.clone(),
            results: Rc::new(RefCell::new(vec![
                Ok(response(Some("chat-1"))),
                Ok(response(Some("chat-2"))),
            ])),
        };
        let mut controller = ConversationController::new(history.clone());
        controller.connect(Connection { query, redactor }).unwrap();
        controller.submit(submission("first")).await.unwrap();
        controller.submit(submission("second")).await.unwrap();
        assert_eq!(*calls.borrow(), vec![None, Some("chat-1".into())]);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn controller_redacts_query_responses_before_display_and_persistence() {
        let (history, directory) = history("response-redaction");
        let secret = "top-secret";
        let query = Query {
            calls: Rc::new(RefCell::new(vec![])),
            results: Rc::new(RefCell::new(vec![Ok(TranscriptResponse {
                text_blocks: vec![TextBlock {
                    text: format!("Bearer {secret}"),
                }],
                structured_result: Some(serde_json::json!({"token": secret})),
                remote_chat_id: Some(secret.into()),
            })])),
        };
        let mut controller = ConversationController::new(history.clone());
        controller
            .connect(Connection {
                query,
                redactor: Redactor::new(secret),
            })
            .unwrap();

        let SubmitOutcome::Response(display) =
            controller.submit(submission("prompt")).await.unwrap()
        else {
            panic!("response persistence should succeed");
        };
        assert!(!format!("{display:?}").contains(secret));
        assert!(!format!("{:?}", history.list().unwrap()).contains(secret));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn finishing_disconnects_before_another_prompt_can_be_appended() {
        let (history, directory) = history("finish-disconnects");
        let mut controller = ConversationController::new(history.clone());
        controller
            .connect(Connection {
                query: Query {
                    calls: Rc::new(RefCell::new(vec![])),
                    results: Rc::new(RefCell::new(vec![Ok(response(Some("chat-1")))])),
                },
                redactor: Redactor::default(),
            })
            .unwrap();
        controller.submit(submission("first")).await.unwrap();
        controller.finish().unwrap();

        assert!(controller.needs_connection());
        assert!(controller.submit(submission("second")).await.is_err());
        assert!(
            history.list().unwrap().iter().all(
                |session| session.status() == &crate::session_history::SessionStatus::Completed
            )
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn failed_finish_keeps_the_active_session_submittable() {
        let (history, directory) = history("failed-finish");
        let mut controller = ConversationController::new(history.clone());
        controller
            .connect(Connection {
                query: Query {
                    calls: Rc::new(RefCell::new(vec![])),
                    results: Rc::new(RefCell::new(vec![
                        Ok(response(Some("chat-1"))),
                        Ok(response(Some("chat-2"))),
                    ])),
                },
                redactor: Redactor::default(),
            })
            .unwrap();
        controller.submit(submission("first")).await.unwrap();

        std::fs::remove_dir_all(&directory).unwrap();
        std::fs::write(&directory, "not a directory").unwrap();
        assert!(controller.finish().is_err());
        std::fs::remove_file(&directory).unwrap();

        controller.submit(submission("second")).await.unwrap();
        assert!(
            history
                .list()
                .unwrap()
                .iter()
                .all(|session| session.status() == &crate::session_history::SessionStatus::Active)
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
/// One Chat Prompt in its two intentional forms: the user's input for Local Adapt History and
/// the resolved message sent to Adapt.
pub struct PromptSubmission {
    original: String,
    outbound: String,
}

impl PromptSubmission {
    pub fn unchanged(prompt: impl Into<String>) -> Self {
        let original = prompt.into();
        Self {
            outbound: original.clone(),
            original,
        }
    }

    pub(crate) fn expanded(original: impl Into<String>, outbound: String) -> Self {
        Self {
            original: original.into(),
            outbound,
        }
    }

    pub fn outbound(&self) -> &str {
        &self.outbound
    }
}
