use adapt_tui::{
    adapt_client::{AdaptClient, QueryResponse},
    chat_terminal::{Repl, ReplCommand, parse_command},
    config,
    session_history::{Session, SessionEntryKind, SessionHistory},
};
use anyhow::Result;
use clap::Parser;
use rmcp::model::RawContent;

const ASK_ADAPT_WARNING: &str = "warning: ask_adapt is not verified as read-only and may perform mutations; use only for development investigations";
const RESUME_REQUIRES_DEVELOPMENT_MODE: &str = "Remote continuation is available only with --allow-unverified-ask-adapt because it uses Adapt's unverified ask_adapt capability.";

struct ResumeTarget {
    session_id: String,
    remote_chat_id: Option<String>,
}

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "adapt-tui",
    version,
    about = "A read-only terminal client for Adapt's MCP server"
)]
struct Cli {
    /// Enable the unverified ask_adapt capability for development investigations.
    #[arg(long)]
    allow_unverified_ask_adapt: bool,

    /// Natural-language prompt to submit.
    #[arg(value_name = "PROMPT")]
    prompt: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    if !args.prompt.is_empty() {
        let config = config::load()?;
        let client = AdaptClient::connect(&config).await?;
        client.discover_capabilities().await?;
        return run_prompt(&client, &args).await;
    }
    run_terminal(args.allow_unverified_ask_adapt).await
}

async fn run_prompt(client: &AdaptClient, args: &Cli) -> Result<()> {
    if args.allow_unverified_ask_adapt {
        eprintln!("{ASK_ADAPT_WARNING}");
    }
    let prompt = args.prompt.join(" ");
    let response = if args.allow_unverified_ask_adapt {
        client.query_ask_adapt(&prompt, true).await?
    } else {
        client.query_read_only(&prompt).await?
    };
    println!("response: {}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn run_terminal(allow_unverified_ask_adapt: bool) -> Result<()> {
    let mut repl = Repl::start(allow_unverified_ask_adapt)?;
    let offline_history = SessionHistory::for_credential_file(config::default_config_path()?, "");
    let mut client = None;
    let mut history: Option<SessionHistory> = None;
    let mut session: Option<Session> = None;
    let mut remote_chat_id = None;
    let mut resume_target: Option<ResumeTarget> = None;
    loop {
        let Some(prompt) = repl.read_prompt()? else {
            if let (Some(history), Some(session)) = (&history, &mut session) {
                history.complete(session)?;
            }
            return Ok(());
        };
        if prompt.is_empty() {
            continue;
        }
        if let Some(command) = parse_command(&prompt) {
            match command {
                ReplCommand::History => show_history(&mut repl, &offline_history)?,
                ReplCommand::Open(id) => {
                    if let Some(opened) = load_history(&mut repl, &offline_history, &id)? {
                        if let (Some(history), Some(session)) = (&history, &mut session) {
                            history.complete(session)?;
                        }
                        remote_chat_id = if allow_unverified_ask_adapt {
                            opened.remote_chat_id.clone()
                        } else {
                            None
                        };
                        resume_target = Some(ResumeTarget {
                            session_id: opened.id.clone(),
                            remote_chat_id: opened.remote_chat_id.clone(),
                        });
                        session = None;
                        repl.clear_transcript()?;
                        render_history(&mut repl, opened)?;
                        if resume_target
                            .as_ref()
                            .is_some_and(|target| target.remote_chat_id.is_none())
                        {
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
        if client.is_none() {
            let config = match config::load() {
                Ok(config) => config,
                Err(error) => {
                    repl.show_error(&error.to_string())?;
                    continue;
                }
            };
            let connected = match AdaptClient::connect(&config).await {
                Ok(client) => match client.discover_capabilities().await {
                    Ok(_) => client,
                    Err(error) => {
                        repl.show_error(&error.to_string())?;
                        continue;
                    }
                },
                Err(error) => {
                    repl.show_error(&error.to_string())?;
                    continue;
                }
            };
            let session_history =
                SessionHistory::for_credential_file(&config.source, &config.bearer_token);
            history = Some(session_history);
            client = Some(connected);
        }
        if session.is_none() {
            let history = history.as_ref().expect("initialized with the client");
            session = Some(if let Some(target) = resume_target.take() {
                history.create_continuation(
                    Some(&target.session_id),
                    target.remote_chat_id.as_deref(),
                )?
            } else {
                history.create()?
            });
        }
        let client = client.as_ref().expect("initialized above");
        if let (Some(history), Some(session)) = (&history, &mut session) {
            history.append_prompt(session, &prompt)?;
        }
        repl.show_working()?;
        let result = if allow_unverified_ask_adapt {
            client
                .query_ask_adapt_in_conversation(&prompt, remote_chat_id.as_deref(), true)
                .await
        } else {
            client.query_read_only(&prompt).await
        };
        match result {
            Ok(response) => {
                if let Some(chat_id) = response.chat_id.clone() {
                    remote_chat_id = Some(chat_id);
                }
                if let (Some(history), Some(session)) = (&history, &mut session) {
                    append_history_response(history, session, &response)?;
                }
                append_response(&mut repl, response)?;
            }
            Err(error) => repl.show_error(&error.to_string())?,
        }
    }
}

fn append_history_response(
    history: &SessionHistory,
    session: &mut Session,
    response: &QueryResponse,
) -> Result<()> {
    let content = response
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    history.append_response(
        session,
        content,
        response.structured_content.clone(),
        response.chat_id.clone(),
    )?;
    Ok(())
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
    let session = match history.load(id) {
        Ok(session) => session,
        Err(error) => {
            repl.show_error(&error.to_string())?;
            return Ok(None);
        }
    };
    Ok(Some(session))
}

fn render_history(repl: &mut Repl, session: Session) -> Result<()> {
    repl.show_notice(&format!(
        "Session {} · {}",
        session.id,
        session.status.display_name()
    ))?;
    for entry in session.entries {
        match entry.kind {
            SessionEntryKind::Prompt { text } => repl.show_you(&text)?,
            SessionEntryKind::Response {
                content,
                structured_result,
                ..
            } => {
                for item in content {
                    repl.show_adapt(&stored_content(&item))?;
                }
                if let Some(value) = structured_result {
                    repl.show_structured_result(value)?;
                }
            }
        }
    }
    Ok(())
}

fn stored_content(value: &serde_json::Value) -> String {
    value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            serde_json::to_string(value)
                .unwrap_or_else(|_| "[unrenderable response content]".into())
        })
}

fn compact(text: &str) -> String {
    text.chars().take(72).collect()
}

fn append_response(repl: &mut Repl, response: QueryResponse) -> std::io::Result<()> {
    for content in response.content {
        let content = match &content.raw {
            RawContent::Text(text) => text.text.clone(),
            _ => serde_json::to_string(&content)
                .unwrap_or_else(|_| "[unrenderable response content]".to_owned()),
        };
        repl.show_adapt(&content)?;
    }
    if let Some(structured_content) = response.structured_content {
        repl.show_structured_result(structured_content)?;
    }
    repl.finish_response()
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use adapt_tui::{adapt_client::QueryResponse, chat_terminal::redact_text};
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
            Cli::try_parse_from([
                "adapt-tui",
                "--allow-unverified-ask-adapt",
                "find",
                "incidents",
            ])
            .unwrap(),
            Cli {
                prompt: vec!["find".to_owned(), "incidents".to_owned()],
                allow_unverified_ask_adapt: true,
            }
        );
    }

    #[test]
    fn flag_alone_does_not_create_a_prompt() {
        assert_eq!(
            Cli::try_parse_from(["adapt-tui", "--allow-unverified-ask-adapt"]).unwrap(),
            Cli {
                prompt: vec![],
                allow_unverified_ask_adapt: true,
            }
        );
    }

    #[test]
    fn help_flags_are_handled_by_clap() {
        for flag in ["--help", "-h"] {
            let error = Cli::try_parse_from(["adapt-tui", flag]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::DisplayHelp);
            assert!(error.to_string().contains("Usage: adapt-tui"));
            assert!(error.to_string().contains("--allow-unverified-ask-adapt"));
        }
    }

    #[test]
    fn version_flag_is_handled_by_clap() {
        let error = Cli::try_parse_from(["adapt-tui", "--version"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayVersion);
        assert!(error.to_string().starts_with("adapt-tui "));
    }

    #[test]
    fn unknown_options_are_rejected() {
        let error = Cli::try_parse_from(["adapt-tui", "--unknown"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn warning_is_explicit_about_mutations() {
        assert!(super::ASK_ADAPT_WARNING.contains("not verified as read-only"));
        assert!(super::ASK_ADAPT_WARNING.contains("may perform mutations"));
    }

    #[test]
    fn text_mcp_content_can_be_extracted_for_the_repl() {
        let response = QueryResponse {
            content: vec![Content::new(
                RawContent::text("Hi Guilherme! What can I help with today?"),
                None,
            )],
            structured_content: None,
            chat_id: None,
        };
        let content = match &response.content[0].raw {
            RawContent::Text(text) => text.text.clone(),
            _ => unreachable!("test response contains text"),
        };
        assert_eq!(content, "Hi Guilherme! What can I help with today?");
        assert_eq!(redact_text("Bearer secret"), "Bearer [redacted]");
    }

    #[test]
    fn saved_text_content_is_rendered_as_text() {
        assert_eq!(
            super::stored_content(&serde_json::json!({"type": "text", "text": "Saved reply"})),
            "Saved reply"
        );
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
