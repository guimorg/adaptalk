use adapt_tui::{
    adapt_client::{AdaptClient, QueryResponse},
    chat_terminal::Repl,
    config,
};
use anyhow::Result;
use clap::Parser;
use rmcp::model::RawContent;

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
    let client: Result<AdaptClient> = async {
        let config = config::load()?;
        let client = AdaptClient::connect(&config).await?;
        client.discover_capabilities().await?;
        Ok(client)
    }
    .await;
    let client = match client {
        Ok(client) => client,
        Err(error) => {
            repl.show_error(&error.to_string())?;
            return Ok(());
        }
    };

    let mut remote_chat_id = None;
    loop {
        let Some(prompt) = repl.read_prompt()? else {
            return Ok(());
        };
        if prompt.is_empty() {
            continue;
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
                append_response(&mut repl, response)?;
            }
            Err(error) => repl.show_error(&error.to_string())?,
        }
    }
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
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
