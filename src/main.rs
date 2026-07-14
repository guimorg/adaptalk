use adapt_tui::{
    adapt_client::{AdaptClient, QueryResponse},
    chat_terminal::{ChatState, ClientEvent, TerminalSession, is_exit_key},
    config,
};
use anyhow::Result;
use clap::Parser;
use crossterm::event::KeyCode;

const ASK_ADAPT_WARNING: &str = "warning: ask_adapt is not verified as read-only and may perform mutations; use only for development investigations";

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
    if args.allow_unverified_ask_adapt {
        println!("{ASK_ADAPT_WARNING}");
    }
    if !args.prompt.is_empty() {
        let config = config::load()?;
        let client = AdaptClient::connect(&config).await?;
        client.discover_capabilities().await?;
        return run_prompt(&client, &args).await;
    }
    run_terminal(args.allow_unverified_ask_adapt).await
}

async fn run_prompt(client: &AdaptClient, args: &Cli) -> Result<()> {
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
    let mut terminal = TerminalSession::enter()?;
    let mut state = ChatState::new();
    terminal.draw(&state)?;
    let client: Result<AdaptClient> = async {
        let config = config::load()?;
        let client = AdaptClient::connect(&config).await?;
        client.discover_capabilities().await?;
        Ok(client)
    }
    .await;
    let client = match client {
        Ok(client) => {
            state.set_ready();
            client
        }
        Err(error) => {
            state.apply(ClientEvent::Error(error.to_string()));
            return wait_for_exit(&mut terminal, &state);
        }
    };

    loop {
        terminal.draw(&state)?;
        let Some(key) = terminal.next_key()? else {
            continue;
        };
        if is_exit_key(key) {
            return Ok(());
        }
        match key.code {
            KeyCode::Char(character) if key.modifiers.is_empty() => state.push_input(character),
            KeyCode::Backspace => state.pop_input(),
            KeyCode::Up => state.scroll_up(),
            KeyCode::Down => state.scroll_down(),
            KeyCode::Enter => {
                let Some(prompt) = state.take_prompt() else {
                    continue;
                };
                state.apply(ClientEvent::PromptSubmitted(prompt.clone()));
                state.apply(ClientEvent::ResponseStarted);
                terminal.draw(&state)?;
                let result = if allow_unverified_ask_adapt {
                    client.query_ask_adapt(&prompt, true).await
                } else {
                    client.query_read_only(&prompt).await
                };
                match result {
                    Ok(response) => append_response(&mut state, response),
                    Err(error) => state.apply(ClientEvent::Error(error.to_string())),
                }
            }
            _ => {}
        }
    }
}

fn wait_for_exit(terminal: &mut TerminalSession, state: &ChatState) -> Result<()> {
    loop {
        terminal.draw(state)?;
        if let Some(key) = terminal.next_key()?
            && is_exit_key(key)
        {
            return Ok(());
        }
    }
}

fn append_response(state: &mut ChatState, response: QueryResponse) {
    for content in response.content {
        let content = serde_json::to_string(&content)
            .unwrap_or_else(|_| "[unrenderable response content]".to_owned());
        state.apply(ClientEvent::ResponseChunk(content));
    }
    if let Some(structured_content) = response.structured_content {
        state.apply(ClientEvent::StructuredResult(structured_content));
    }
    state.apply(ClientEvent::ResponseCompleted);
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::{CommandFactory, Parser, error::ErrorKind};

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
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
