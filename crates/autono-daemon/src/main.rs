use anyhow::Result;
use autono_core::config::{Config, Paths};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let paths = Paths::discover(None)?;
    let config = Config::load(&paths).await?;
    let stop = CancellationToken::new();
    let signal = stop.clone();
    tokio::spawn(async move {
        if let Err(error) = autono_daemon::shutdown_signal().await {
            tracing::error!(%error, "signal handler failed");
        }
        signal.cancel();
    });
    autono_daemon::run(paths, config, stop).await
}
