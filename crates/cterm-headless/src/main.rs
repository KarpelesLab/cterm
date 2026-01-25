//! ctermd - Headless terminal daemon with gRPC API

use cterm_headless::cli::Cli;
use cterm_headless::server::run_server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse_args();

    // Initialize logging
    let log_level = cli.log_level.parse().unwrap_or(log::LevelFilter::Info);
    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp_secs()
        .init();

    log::info!("ctermd starting...");

    // Run the server
    let config = cli.to_server_config();
    run_server(config).await?;

    Ok(())
}
