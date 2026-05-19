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
    let loaded_blocks = server::load_persistent_state()
        .await
        .context("failed to load persisted state")?;
    info!(blocks = loaded_blocks, "loaded persisted world edits");
    server::spawn_persistence_task();

    let listener = TcpListener::bind(&config.addr)
        .await
        .with_context(|| format!("failed to bind {}", config.addr))?;

    info!(addr = %config.addr, "VibeCraft server listening");

    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let config = config.clone();

                tokio::spawn(async move {
                    if let Err(err) = server::handle_connection(stream, config).await {
                        debug!(%peer, error = %err, "connection closed");
                    }
                });
            }
            result = &mut shutdown => {
                result?;
                info!("shutdown signal received, saving persisted state");
                server::save_persistent_state().await?;
                return Ok(());
            }
        }
    }
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        // Restart scripts send SIGTERM, while terminals usually send Ctrl-C.
        let mut terminate =
            signal(SignalKind::terminate()).context("failed to listen for SIGTERM")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("failed to listen for Ctrl-C")?,
            _ = terminate.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to listen for Ctrl-C")?;
    }

    Ok(())
}
