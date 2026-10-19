//! Own connection tasks so forced HTTP teardown actually closes their sockets.
use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use autono_core::Supervisor;
use axum::Router;
use hyper::server::conn::http1;
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinSet};
use tokio_util::sync::CancellationToken;

pub async fn serve(
    listener: TcpListener,
    app: Router,
    supervisor: Supervisor,
    stop: CancellationToken,
    drain_timeout: Duration,
    header_timeout: Duration,
) -> Result<()> {
    let drain = CancellationToken::new();
    let slots = Arc::new(Semaphore::new(128));
    let mut connections = JoinSet::new();
    let result = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break Ok(()),
            result = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = result { tracing::warn!(%error, "HTTP connection task failed"); }
            }
            accepted = listener.accept() => {
                let (socket, _) = match accepted { Ok(pair) => pair, Err(error) => break Err(error).context("accept local connection") };
                let Ok(slot) = slots.clone().try_acquire_owned() else { drop(socket); continue; };
                let service = TowerToHyperService::new(app.clone());
                let signal = drain.clone();
                connections.spawn(async move {
                    let _slot = slot;
                    let mut builder = http1::Builder::new();
                    builder.timer(TokioTimer::new()).header_read_timeout(header_timeout);
                    let connection = builder.serve_connection(TokioIo::new(socket), service);
                    tokio::pin!(connection);
                    let result = tokio::select! {
                        result = &mut connection => result,
                        _ = signal.cancelled() => {
                            connection.as_mut().graceful_shutdown();
                            connection.await
                        }
                    };
                    if let Err(error) = result { tracing::debug!(%error, "local HTTP connection closed"); }
                });
            }
        }
    };
    drop(listener);
    // Control signalling is immediate and independent of SQLite and HTTP work.
    supervisor.signal_shutdown();
    let shutdown = supervisor.shutdown().await;
    // Only after process control/reaping is finished may HTTP clients be cut off.
    drain.cancel();
    if tokio::time::timeout(drain_timeout, async {
        while connections.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    }
    result.and(shutdown)
}
