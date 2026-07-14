use adapt_tui::{adapt_client::AdaptClient, config};
use anyhow::Result;

const ALLOW_UNVERIFIED_ASK_ADAPT: &str = "--allow-unverified-ask-adapt";
const ASK_ADAPT_WARNING: &str = "warning: ask_adapt is not verified as read-only and may perform mutations; use only for development investigations";

#[derive(Debug, PartialEq, Eq)]
struct CliArgs {
    prompt: Option<String>,
    allow_unverified_ask_adapt: bool,
}

fn parse_args<I>(args: I) -> CliArgs
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut allow_unverified_ask_adapt = false;
    let prompt = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .filter(|arg| {
            if arg == ALLOW_UNVERIFIED_ASK_ADAPT {
                allow_unverified_ask_adapt = true;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    CliArgs {
        prompt: (!prompt.is_empty()).then_some(prompt),
        allow_unverified_ask_adapt,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1));
    if args.allow_unverified_ask_adapt {
        println!("{ASK_ADAPT_WARNING}");
    }
    let config = config::load()?;
    println!("configuration: {}", config.source.display());
    let client = AdaptClient::connect(&config).await?;
    println!("connection: connected and initialized");
    let capabilities = client.discover_read_only_capabilities().await?;
    println!("capabilities: {}", capabilities.len());
    for capability in capabilities {
        println!("- {}", capability.name);
    }
    if let Some(prompt) = args.prompt {
        let response = if args.allow_unverified_ask_adapt {
            client.query_ask_adapt(&prompt, true).await?
        } else {
            client.query_read_only(&prompt).await?
        };
        println!("response: {}", serde_json::to_string_pretty(&response)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CliArgs, parse_args};

    #[test]
    fn empty_arguments_do_not_submit_a_prompt() {
        assert_eq!(parse_args(Vec::<String>::new()).prompt, None);
    }

    #[test]
    fn prompt_arguments_are_joined_for_submission() {
        assert_eq!(
            parse_args(["find", "recent", "incidents"]).prompt,
            Some("find recent incidents".to_owned())
        );
    }

    #[test]
    fn opt_in_flag_is_removed_from_prompt() {
        assert_eq!(
            parse_args(["--allow-unverified-ask-adapt", "find", "incidents"]),
            CliArgs {
                prompt: Some("find incidents".to_owned()),
                allow_unverified_ask_adapt: true,
            }
        );
    }

    #[test]
    fn flag_alone_does_not_create_a_prompt() {
        assert_eq!(
            parse_args(["--allow-unverified-ask-adapt"]),
            CliArgs {
                prompt: None,
                allow_unverified_ask_adapt: true,
            }
        );
    }

    #[test]
    fn warning_is_explicit_about_mutations() {
        assert!(super::ASK_ADAPT_WARNING.contains("not verified as read-only"));
        assert!(super::ASK_ADAPT_WARNING.contains("may perform mutations"));
    }
}
