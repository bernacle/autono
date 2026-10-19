pub mod api;
mod ownership;
pub mod server;

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};
use autono_core::{
    Store, Supervisor,
    config::{Config, Paths},
};
use autono_protocol::{API_VERSION, DaemonMetadata};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub async fn run(paths: Paths, config: Config, stop: CancellationToken) -> Result<()> {
    paths.prepare().await?;
    let owner = ownership::Ownership::acquire(&paths).await?;
    let store = Store::open(&paths.database()).await?;
    tokio::time::timeout(std::time::Duration::from_secs(5), store.health())
        .await
        .context("startup database health inspection timed out")??;
    let reconciled = store.reconcile().await?;
    tracing::info!(reconciled, "reconciled unfinished runs");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.daemon.port))
        .await
        .context("bind loopback daemon API")?;
    let address = listener.local_addr()?;
    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);
    let metadata = DaemonMetadata {
        api_version: API_VERSION,
        pid: std::process::id(),
        address,
        token: URL_SAFE_NO_PAD.encode(secret),
    };
    owner.publish(&paths, &metadata).await?;
    let supervisor = Supervisor::configured(store, &config.runner);
    let app = api::configured_router(
        supervisor.clone(),
        metadata.token.clone(),
        address,
        stop.clone(),
        std::time::Duration::from_secs(config.daemon.body_timeout_seconds),
    );
    tracing::info!(%address, home = %paths.home.display(), "Autono daemon ready");
    let result = server::serve(
        listener,
        app,
        supervisor,
        stop,
        std::time::Duration::from_secs(config.daemon.drain_timeout_seconds),
        std::time::Duration::from_secs(config.daemon.body_timeout_seconds),
    )
    .await;
    owner.unpublish(&paths).await?;
    result
}

pub async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("listen for Ctrl-C")?,
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.context("listen for Ctrl-C")?;
    Ok(())
}

pub fn validate_address(address: SocketAddr) -> Result<()> {
    anyhow::ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "daemon metadata must contain a loopback address and nonzero port"
    );
    Ok(())
}
