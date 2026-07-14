use adapt_tui::{adapt_client::AdaptClient, config};
use anyhow::Result;

fn prompt_from<I>(args: I) -> Option<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let prompt = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>()
        .join(" ");
    (!prompt.is_empty()).then_some(prompt)
}

fn prompt_from_args() -> Option<String> {
    prompt_from(std::env::args().skip(1))
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = config::load()?;
    println!("configuration: {}", config.source.display());
    let client = AdaptClient::connect(&config).await?;
    println!("connection: connected and initialized");
    let capabilities = client.discover_read_only_capabilities().await?;
    println!("capabilities: {}", capabilities.len());
    for capability in capabilities {
        println!("- {}", capability.name);
    }
    if let Some(prompt) = prompt_from_args() {
        let response = client.query_read_only(&prompt).await?;
        println!("response: {}", serde_json::to_string_pretty(&response)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::prompt_from;

    #[test]
    fn empty_arguments_do_not_submit_a_prompt() {
        assert_eq!(prompt_from(Vec::<String>::new()), None);
    }

    #[test]
    fn prompt_arguments_are_joined_for_submission() {
        assert_eq!(
            prompt_from(["find", "recent", "incidents"]),
            Some("find recent incidents".to_owned())
        );
    }
}
