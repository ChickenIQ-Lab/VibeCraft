mod constants;
mod cursor;
mod protocol;
mod server;
mod types;

use crate::types::Config;
use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("vibecraft=info".parse()?))
        .init();

    let config = Config::from_env();
    let listener = TcpListener::bind(&config.addr)
        .await
        .with_context(|| format!("failed to bind {}", config.addr))?;

    info!(addr = %config.addr, "VibeCraft server listening");

    loop {
        let (stream, peer) = listener.accept().await?;
        let config = config.clone();

        tokio::spawn(async move {
            if let Err(err) = server::handle_connection(stream, config).await {
                debug!(%peer, error = %err, "connection closed");
            }
        });
    }
}
