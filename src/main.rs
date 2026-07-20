use adaptalk::{
    adapt_client::AdaptClient,
    chat_terminal::{Repl, ReplCommand, parse_command},
    config,
    conversation_controller::{
        Connection, ConversationController, ConversationQuery, QueryFuture, SubmitOutcome,
    },
    file_references::FileReferenceResolver,
    redaction::Redactor,
    session_history::{Session, SessionEntryKind, SessionHistory},
    transcript::{self, TranscriptResponse},
    update_checker,
};
use anyhow::Result;
use clap::Parser;
use std::time::Duration;

const ASK_ADAPT_WARNING: &str = "warning: ask_adapt is not verified as read-only and may perform mutations; use only for development investigations";
const RESUME_REQUIRES_DEVELOPMENT_MODE: &str = "Remote continuation is available only with --allow-unverified-ask-adapt because it uses Adapt's unverified ask_adapt capability.";
const CONTINUATION_FALLBACK_NOTICE: &str = "Remote continuation failed; the previous session may have expired. The next prompt starts a fresh Adapt session.";

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "adaptalk",
    version,
    about = "A read-only terminal client for Adapt's MCP server"
)]
struct Cli {
    #[arg(long)]
    allow_unverified_ask_adapt: bool,
    #[arg(value_name = "PROMPT")]
    prompt: Vec<String>,
}

struct TerminalQuery {
    client: AdaptClient,
    allow_unverified_ask_adapt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponsePresentation {
    Immediate,
    SimulatedStream { delay: Duration },
}

impl ResponsePresentation {
    fn toggle(self, delay: Duration) -> Self {
        match self {
            Self::Immediate => Self::SimulatedStream { delay },
            Self::SimulatedStream { .. } => Self::Immediate,
        }
    }

    fn with_delay(self, delay: Duration) -> Self {
        match self {
            Self::Immediate => Self::Immediate,
            Self::SimulatedStream { .. } => Self::SimulatedStream { delay },
        }
    }
}

struct TerminalConnection {
    connection: Connection<TerminalQuery>,
    stream_delay: Duration,
}

impl ConversationQuery for TerminalQuery {
    fn query<'a>(&'a self, prompt: &'a str, continuation: Option<&'a str>) -> QueryFuture<'a> {
        Box::pin(async move {
            let response = if self.allow_unverified_ask_adapt {
                self.client
                    .query_ask_adapt_in_conversation(prompt, continuation, true)
                    .await?
            } else {
                self.client.query_read_only(prompt).await?
            };
            Ok(transcript::from_query_response(response))
        })
    }
}

async fn connect_terminal(allow_unverified_ask_adapt: bool) -> Result<TerminalConnection> {
    let config = config::load()?;
    let stream_delay = config.stream_delay;
    let client = AdaptClient::connect(&config).await?;
    client.discover_capabilities().await?;
    let redactor = Redactor::new(&config.bearer_token);
    Ok(TerminalConnection {
        connection: Connection {
            query: TerminalQuery {
                client,
                allow_unverified_ask_adapt,
            },
            redactor,
        },
        stream_delay,
    })
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
    let TerminalConnection {
        connection: Connection { query, redactor },
        ..
    } = connect_terminal(args.allow_unverified_ask_adapt).await?;
    if args.allow_unverified_ask_adapt {
        eprintln!("{}", redactor.text(ASK_ADAPT_WARNING));
    }
    let prompt = args.prompt.join(" ");
    let response = query
        .query(&prompt, None)
        .await
        .map_err(|e| anyhow::anyhow!("{}", redactor.text(&e.to_string())))?;
    println!(
        "response: {}",
        serde_json::to_string_pretty(
            &redactor
                .transcript_response(response)
                .into_inner()
                .display_value(),
        )?
    );
    Ok(())
}

async fn run_terminal(allow_unverified_ask_adapt: bool) -> Result<()> {
    let mut repl = Repl::start(allow_unverified_ask_adapt)?;
    let mut updates = update_checker::spawn();
    let history = SessionHistory::for_credential_file(config::default_config_path()?);
    let mut controller = ConversationController::<TerminalQuery>::new(history);
    let mut presentation = ResponsePresentation::SimulatedStream {
        delay: Duration::from_millis(config::DEFAULT_STREAM_DELAY_MS),
    };
    let mut stream_delay = Duration::from_millis(config::DEFAULT_STREAM_DELAY_MS);
    loop {
        if let Ok(Ok(Some(notice))) = updates.try_recv() {
            repl.show_update(notice)?;
        }
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
                        let opened = controller.open(session)?;
                        repl.clear_transcript()?;
                        render_history(&mut repl, &opened).await?;
                        match controller.viewing_continuation()? {
                            None => {
                                repl.show_notice("This session has no remote chat ID; the next prompt starts a new remote conversation.")?;
                            }
                            Some(_) if !allow_unverified_ask_adapt => {
                                repl.show_notice(RESUME_REQUIRES_DEVELOPMENT_MODE)?;
                            }
                            Some(chat_id) => {
                                repl.show_notice(&format!(
                                    "Remote session {chat_id} available; the next prompt will attempt to resume the Adapt session."
                                ))?;
                            }
                        }
                    }
                }
                ReplCommand::ToggleStreaming => {
                    presentation = presentation.toggle(stream_delay);
                    repl.show_notice(
                        if matches!(presentation, ResponsePresentation::SimulatedStream { .. }) {
                            "Mock response streaming enabled."
                        } else {
                            "Mock response streaming disabled."
                        },
                    )?;
                }
                ReplCommand::Unknown(message) => repl.show_error(&message)?,
            }
            continue;
        }
        let submission = match FileReferenceResolver::for_current_dir()
            .and_then(|resolver| resolver.resolve(&prompt))
        {
            Ok(submission) => submission,
            Err(error) => {
                repl.show_error(&format!("prompt error: {error}"))?;
                continue;
            }
        };
        repl.show_working()?;
        if controller.needs_connection() {
            let terminal_connection = match connect_terminal(allow_unverified_ask_adapt).await {
                Ok(connection) => connection,
                Err(error) => {
                    repl.show_error(&controller.redact(&error.to_string()))?;
                    continue;
                }
            };
            stream_delay = terminal_connection.stream_delay;
            presentation = presentation.with_delay(stream_delay);
            if let Err(error) = controller.connect(terminal_connection.connection) {
                repl.show_error(&controller.redact(&error.to_string()))?;
                continue;
            }
        }
        let result = controller.submit(submission).await;
        match result {
            Ok(SubmitOutcome::Response(response)) => {
                render_response(&mut repl, response, presentation).await?
            }
            Ok(SubmitOutcome::ResponseWithPersistenceWarning { response, error }) => {
                render_response(&mut repl, response, presentation).await?;
                repl.show_notice(&format!(
                    "warning: response was received but could not be saved locally: {error}"
                ))?;
            }
            Ok(SubmitOutcome::ErrorWithPersistenceWarning {
                error,
                persistence_error,
            }) => {
                repl.show_error(&controller.redact(&error.to_string()))?;
                repl.show_notice(&format!(
                    "warning: the error could not be saved locally: {persistence_error}"
                ))?;
            }
            Ok(SubmitOutcome::ContinuationFailed { error }) => {
                repl.show_error(&controller.redact(&error.to_string()))?;
                repl.show_notice(CONTINUATION_FALLBACK_NOTICE)?;
            }
            Ok(SubmitOutcome::ContinuationFailedWithPersistenceWarning {
                error,
                persistence_error,
            }) => {
                repl.show_error(&controller.redact(&error.to_string()))?;
                repl.show_notice(CONTINUATION_FALLBACK_NOTICE)?;
                repl.show_notice(&format!(
                    "warning: the error could not be saved locally: {persistence_error}"
                ))?;
            }
            Err(error) => repl.show_error(&controller.redact(&error.to_string()))?,
        }
    }
}

fn show_history(repl: &mut Repl, history: &SessionHistory) -> Result<()> {
    let sessions = history.list()?;
    if sessions.is_empty() {
        return Ok(repl.show_notice("No local sessions saved yet.")?);
    }
    for session in sessions {
        let prompt = session
            .entries()
            .iter()
            .find_map(|entry| match entry.kind() {
                SessionEntryKind::Prompt { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("(no prompts)");
        repl.show_notice(&format!(
            "{} · {} · {}",
            session.id(),
            session.status().display_name(),
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
async fn render_history(repl: &mut Repl, session: &Session) -> Result<()> {
    repl.show_notice(&format!(
        "Session {} · {}",
        session.id(),
        session.status().display_name()
    ))?;
    for entry in session.entries() {
        match entry.kind() {
            SessionEntryKind::Prompt { text } => repl.show_you(text)?,
            SessionEntryKind::Response(response) => {
                render_response(repl, response.clone(), ResponsePresentation::Immediate).await?
            }
            SessionEntryKind::Error { message } => repl.show_error(message)?,
        }
    }
    Ok(())
}
async fn render_response(
    repl: &mut Repl,
    response: TranscriptResponse,
    presentation: ResponsePresentation,
) -> Result<()> {
    for block in response.text_blocks {
        match presentation {
            ResponsePresentation::Immediate => repl.show_adapt(&block.text)?,
            ResponsePresentation::SimulatedStream { delay } => {
                repl.show_adapt_streaming(&block.text, delay).await?
            }
        }
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
    use std::time::Duration;

    use super::{Cli, ResponsePresentation};
    use adaptalk::{adapt_client::QueryResponse, redaction::Redactor, transcript};
    use clap::{CommandFactory, Parser, error::ErrorKind};
    use rmcp::model::{Content, RawContent};
    #[test]
    fn empty_arguments_do_not_submit_a_prompt() {
        assert!(Cli::try_parse_from(["adaptalk"]).unwrap().prompt.is_empty());
    }
    #[test]
    fn prompt_arguments_are_joined_for_submission() {
        assert_eq!(
            Cli::try_parse_from(["adaptalk", "find", "recent", "incidents"])
                .unwrap()
                .prompt
                .join(" "),
            "find recent incidents"
        );
    }
    #[test]
    fn opt_in_flag_is_removed_from_prompt() {
        assert_eq!(
            Cli::try_parse_from(["adaptalk", "--allow-unverified-ask-adapt", "find"]).unwrap(),
            Cli {
                prompt: vec!["find".into()],
                allow_unverified_ask_adapt: true
            }
        );
    }
    #[test]
    fn help_version_and_unknown_flags_are_handled_by_clap() {
        assert_eq!(
            Cli::try_parse_from(["adaptalk", "--help"])
                .unwrap_err()
                .kind(),
            ErrorKind::DisplayHelp
        );
        assert_eq!(
            Cli::try_parse_from(["adaptalk", "--version"])
                .unwrap_err()
                .kind(),
            ErrorKind::DisplayVersion
        );
        assert_eq!(
            Cli::try_parse_from(["adaptalk", "--unknown"])
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
        let transcript = Redactor::new("top-secret")
            .transcript_response(transcript::from_query_response(response))
            .into_inner();
        let stored = transcript.display_value().to_string();
        assert!(!stored.contains("top-secret"));
    }

    #[test]
    fn stream_toggle_switches_between_immediate_and_simulated_output() {
        let delay = Duration::from_millis(120);
        let simulated = ResponsePresentation::SimulatedStream { delay };
        assert_eq!(simulated.toggle(delay), ResponsePresentation::Immediate);
        assert_eq!(ResponsePresentation::Immediate.toggle(delay), simulated);
    }

    #[test]
    fn configured_delay_updates_only_simulated_output() {
        let configured = Duration::from_millis(80);
        assert_eq!(
            ResponsePresentation::SimulatedStream {
                delay: Duration::from_millis(35),
            }
            .with_delay(configured),
            ResponsePresentation::SimulatedStream { delay: configured }
        );
        assert_eq!(
            ResponsePresentation::Immediate.with_delay(configured),
            ResponsePresentation::Immediate
        );
    }

    #[test]
    fn configured_delay_survives_disabling_and_reenabling_streaming() {
        let configured = Duration::from_millis(120);
        let mut presentation = ResponsePresentation::SimulatedStream { delay: configured };

        presentation = presentation.toggle(configured);
        assert_eq!(presentation, ResponsePresentation::Immediate);

        presentation = presentation.toggle(configured);
        assert_eq!(
            presentation,
            ResponsePresentation::SimulatedStream { delay: configured }
        );
    }
}
