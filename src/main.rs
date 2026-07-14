use adapt_tui::{adapt_client::AdaptClient, config};
use anyhow::Result;

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
    Ok(())
}
