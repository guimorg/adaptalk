use adapt_tui::{
    adapt_client::{AdaptClient, QueryResponse},
    chat_terminal::{Repl, ReplCommand, parse_command},
    config,
    redaction::Redactor,
    session_history::{
        Citation, Session, SessionEntryKind, SessionHistory, TextBlock, TranscriptResponse,
    },
};
use anyhow::Result;
use clap::Parser;
use rmcp::model::RawContent;
use serde_json::Value;

const ASK_ADAPT_WARNING: &str = "warning: ask_adapt is not verified as read-only and may perform mutations; use only for development investigations";
const RESUME_REQUIRES_DEVELOPMENT_MODE: &str = "Remote continuation is available only with --allow-unverified-ask-adapt because it uses Adapt's unverified ask_adapt capability.";

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "adapt-tui",
    version,
    about = "A read-only terminal client for Adapt's MCP server"
)]
struct Cli {
    #[arg(long)]
    allow_unverified_ask_adapt: bool,
    #[arg(value_name = "PROMPT")]
    prompt: Vec<String>,
}

enum TerminalState {
    Disconnected,
    Connected(Box<ActiveConversation>),
    ViewingHistory(ResumeTarget),
}

struct ActiveConversation {
    client: AdaptClient,
    history: SessionHistory,
    session: Session,
    remote_chat_id: Option<String>,
}

struct ResumeTarget {
    session: Session,
}

struct ConversationController {
    state: TerminalState,
    offline_history: SessionHistory,
    redactor: Redactor,
    allow_unverified_ask_adapt: bool,
}

impl ConversationController {
    fn new(offline_history: SessionHistory, allow_unverified_ask_adapt: bool) -> Self {
        Self {
            state: TerminalState::Disconnected,
            offline_history,
            redactor: Redactor::default(),
            allow_unverified_ask_adapt,
        }
    }
    fn history(&self) -> &SessionHistory {
        &self.offline_history
    }
    fn redactor(&self) -> Redactor {
        self.redactor.clone()
    }
    fn finish(&mut self) -> Result<()> {
        if let TerminalState::Connected(active) = &mut self.state {
            active.history.complete(&mut active.session)?;
        }
        Ok(())
    }
    fn open(&mut self, opened: Session) -> Result<()> {
        self.finish()?;
        self.state = TerminalState::ViewingHistory(ResumeTarget { session: opened });
        Ok(())
    }
    fn opened_session(&self) -> Option<&Session> {
        match &self.state {
            TerminalState::ViewingHistory(target) => Some(&target.session),
            _ => None,
        }
    }
    async fn submit(&mut self, prompt: &str) -> Result<TranscriptResponse> {
        self.connect_for_prompt().await?;
        let TerminalState::Connected(active) = &mut self.state else {
            unreachable!("connect_for_prompt always enters Connected")
        };
        active
            .history
            .append_prompt(&mut active.session, self.redactor.text(prompt))?;
        let result = if self.allow_unverified_ask_adapt {
            active
                .client
                .query_ask_adapt_in_conversation(prompt, active.remote_chat_id.as_deref(), true)
                .await
        } else {
            active.client.query_read_only(prompt).await
        };
        match result {
            Ok(response) => {
                let response = transcript_response(response, &self.redactor)?;
                if let Some(chat_id) = response.remote_chat_id.clone() {
                    active.remote_chat_id = Some(chat_id);
                }
                active
                    .history
                    .append_response(&mut active.session, response.clone())?;
                Ok(response)
            }
            Err(error) => {
                let message = self.redactor.text(&error.to_string());
                active
                    .history
                    .append_error(&mut active.session, message.clone())?;
                Err(error.into())
            }
        }
    }
    async fn connect_for_prompt(&mut self) -> Result<()> {
        let target = match std::mem::replace(&mut self.state, TerminalState::Disconnected) {
            TerminalState::Connected(active) => {
                self.state = TerminalState::Connected(active);
                return Ok(());
            }
            TerminalState::ViewingHistory(target) => Some(target),
            TerminalState::Disconnected => None,
        };
        let connection = async {
            let config = config::load()?;
            let client = AdaptClient::connect(&config).await?;
            client.discover_capabilities().await?;
            Ok::<_, anyhow::Error>((config, client))
        }
        .await;
        let (config, client) = match connection {
            Ok(connection) => connection,
            Err(error) => {
                self.state = target
                    .map(TerminalState::ViewingHistory)
                    .unwrap_or(TerminalState::Disconnected);
                return Err(error);
            }
        };
        self.redactor = Redactor::new(&config.bearer_token);
        let history = SessionHistory::for_credential_file(&config.source);
        let session = match target.as_ref() {
            Some(target) => history.create_continuation(
                Some(target.session.id.clone()),
                if self.allow_unverified_ask_adapt {
                    target
                        .session
                        .remote_chat_id
                        .as_ref()
                        .map(|id| self.redactor.text(id))
                } else {
                    None
                },
            )?,
            None => history.create()?,
        };
        self.state = TerminalState::Connected(Box::new(ActiveConversation {
            client,
            history,
            session,
            remote_chat_id: None,
        }));
        if let TerminalState::Connected(active) = &mut self.state
            && self.allow_unverified_ask_adapt
            && let Some(target) = target.as_ref()
        {
            active.remote_chat_id = target.session.remote_chat_id.clone();
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    if !args.prompt.is_empty() {
        return run_prompt(&args).await;
    }
    run_terminal(args.allow_unverified_ask_adapt).await
}

async fn run_prompt(args: &Cli) -> Result<()> {
    let config = config::load()?;
    let redactor = Redactor::new(&config.bearer_token);
    let client = AdaptClient::connect(&config).await?;
    client.discover_capabilities().await?;
    if args.allow_unverified_ask_adapt {
        eprintln!("{}", redactor.text(ASK_ADAPT_WARNING));
    }
    let prompt = args.prompt.join(" ");
    let response = if args.allow_unverified_ask_adapt {
        client.query_ask_adapt(&prompt, true).await?
    } else {
        client.query_read_only(&prompt).await?
    };
    println!(
        "response: {}",
        serde_json::to_string_pretty(&transcript_response(response, &redactor)?)?
    );
    Ok(())
}

async fn run_terminal(allow_unverified_ask_adapt: bool) -> Result<()> {
    let mut repl = Repl::start(allow_unverified_ask_adapt)?;
    let history = SessionHistory::for_credential_file(config::default_config_path()?);
    let mut controller = ConversationController::new(history, allow_unverified_ask_adapt);
    loop {
        let Some(prompt) = repl.read_prompt()? else {
            return controller.finish();
        };
        if prompt.is_empty() {
            continue;
        }
        if let Some(command) = parse_command(&prompt) {
            match command {
                ReplCommand::History => show_history(&mut repl, controller.history())?,
                ReplCommand::Open(id) => {
                    if let Some(session) = load_history(&mut repl, controller.history(), &id)? {
                        controller.open(session)?;
                        repl.clear_transcript()?;
                        let opened = controller
                            .opened_session()
                            .expect("open transition retains the selected transcript");
                        render_history(&mut repl, opened)?;
                        if opened.remote_chat_id.is_none() {
                            repl.show_notice("This session has no remote chat ID; the next prompt starts a new remote conversation.")?;
                        } else if !allow_unverified_ask_adapt {
                            repl.show_notice(RESUME_REQUIRES_DEVELOPMENT_MODE)?;
                        }
                    }
                }
                ReplCommand::Unknown(message) => repl.show_error(&message)?,
            }
            continue;
        }
        repl.show_working()?;
        let result = controller.submit(&prompt).await;
        repl.set_redactor(controller.redactor());
        match result {
            Ok(response) => render_response(&mut repl, response)?,
            Err(error) => repl.show_error(&controller.redactor().text(&error.to_string()))?,
        }
    }
}

fn transcript_response(response: QueryResponse, redactor: &Redactor) -> Result<TranscriptResponse> {
    let mut citations = Vec::new();
    let content = response
        .content
        .into_iter()
        .map(|content| {
            citations.extend(find_citations(&serde_json::to_value(&content)?));
            match content.raw {
                RawContent::Text(text) => Ok(TextBlock {
                    text: redactor.text(&text.text),
                }),
                _ => Ok(TextBlock {
                    text: redactor.text(&serde_json::to_string(&content)?),
                }),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let structured_result = response
        .structured_content
        .map(|value| redactor.value(value));
    citations.extend(
        structured_result
            .as_ref()
            .into_iter()
            .flat_map(find_citations),
    );
    let citations = citations
        .into_iter()
        .map(|value| Citation {
            value: redactor.value(value),
        })
        .collect();
    Ok(TranscriptResponse {
        text_blocks: content,
        structured_result,
        citations,
        remote_chat_id: response.chat_id.map(|id| redactor.text(&id)),
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
                    vec![]
                };
                found.extend(find_citations(value));
                found
            })
            .collect(),
        Value::Array(values) => values.iter().flat_map(find_citations).collect(),
        _ => vec![],
    }
}

fn show_history(repl: &mut Repl, history: &SessionHistory) -> Result<()> {
    let sessions = history.list()?;
    if sessions.is_empty() {
        return Ok(repl.show_notice("No local sessions saved yet.")?);
    }
    for session in sessions {
        let prompt = session
            .entries
            .iter()
            .find_map(|entry| match &entry.kind {
                SessionEntryKind::Prompt { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("(no prompts)");
        repl.show_notice(&format!(
            "{} · {} · {}",
            session.id,
            session.status.display_name(),
            compact(prompt)
        ))?;
    }
    Ok(())
}
fn load_history(repl: &mut Repl, history: &SessionHistory, id: &str) -> Result<Option<Session>> {
    match history.load(id) {
        Ok(session) => Ok(Some(session)),
        Err(error) => {
            repl.show_error(&error.to_string())?;
            Ok(None)
        }
    }
}
fn render_history(repl: &mut Repl, session: &Session) -> Result<()> {
    repl.show_notice(&format!(
        "Session {} · {}",
        session.id,
        session.status.display_name()
    ))?;
    for entry in &session.entries {
        match &entry.kind {
            SessionEntryKind::Prompt { text } => repl.show_you(text)?,
            SessionEntryKind::Response(response) => render_response(repl, response.clone())?,
            SessionEntryKind::Error { message } => repl.show_error(message)?,
        }
    }
    Ok(())
}
fn render_response(repl: &mut Repl, response: TranscriptResponse) -> Result<()> {
    for block in response.text_blocks {
        repl.show_adapt(&block.text)?;
    }
    if let Some(value) = response.structured_result {
        repl.show_structured_result(value)?;
    }
    repl.finish_response()?;
    Ok(())
}
fn compact(text: &str) -> String {
    text.chars().take(72).collect()
}

#[cfg(test)]
mod tests {
    use super::{Cli, transcript_response};
    use adapt_tui::{adapt_client::QueryResponse, redaction::Redactor};
    use clap::{CommandFactory, Parser, error::ErrorKind};
    use rmcp::model::{Content, RawContent};
    #[test]
    fn empty_arguments_do_not_submit_a_prompt() {
        assert!(
            Cli::try_parse_from(["adapt-tui"])
                .unwrap()
                .prompt
                .is_empty()
        );
    }
    #[test]
    fn prompt_arguments_are_joined_for_submission() {
        assert_eq!(
            Cli::try_parse_from(["adapt-tui", "find", "recent", "incidents"])
                .unwrap()
                .prompt
                .join(" "),
            "find recent incidents"
        );
    }
    #[test]
    fn opt_in_flag_is_removed_from_prompt() {
        assert_eq!(
            Cli::try_parse_from(["adapt-tui", "--allow-unverified-ask-adapt", "find"]).unwrap(),
            Cli {
                prompt: vec!["find".into()],
                allow_unverified_ask_adapt: true
            }
        );
    }
    #[test]
    fn help_version_and_unknown_flags_are_handled_by_clap() {
        assert_eq!(
            Cli::try_parse_from(["adapt-tui", "--help"])
                .unwrap_err()
                .kind(),
            ErrorKind::DisplayHelp
        );
        assert_eq!(
            Cli::try_parse_from(["adapt-tui", "--version"])
                .unwrap_err()
                .kind(),
            ErrorKind::DisplayVersion
        );
        assert_eq!(
            Cli::try_parse_from(["adapt-tui", "--unknown"])
                .unwrap_err()
                .kind(),
            ErrorKind::UnknownArgument
        );
        Cli::command().debug_assert();
    }
    #[test]
    fn warning_is_explicit_about_mutations() {
        assert!(super::ASK_ADAPT_WARNING.contains("not verified as read-only"));
        assert!(super::ASK_ADAPT_WARNING.contains("may perform mutations"));
    }
    #[test]
    fn response_translation_redacts_configured_token_for_one_shot_output() {
        let response = QueryResponse {
            content: vec![Content::new(RawContent::text("Bearer top-secret"), None)],
            structured_content: Some(serde_json::json!({"token": "top-secret"})),
            chat_id: None,
        };
        let transcript = transcript_response(response, &Redactor::new("top-secret")).unwrap();
        let stored = serde_json::to_string(&transcript).unwrap();
        assert!(!stored.contains("top-secret"));
    }
}
