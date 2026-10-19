//! Persistence is a bounded consumer, never the process-control loop.
use std::{future::Future, time::Duration};

use anyhow::Result;
use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{EventKind, Store};

#[derive(Clone)]
pub(crate) struct Finalization {
    pub pid: Option<u32>,
    pub launched: Option<DateTime<Utc>>,
    pub result: autono_protocol::ObservedResult,
}

pub(crate) struct OutputReport {
    pub complete: bool,
    pub warning: Option<String>,
}

/// Retry only known transient lock errors. Never cancel an individual commit and
/// then assume it failed: SQLx's worker may still be completing it.
pub(crate) async fn retry<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                let transient = error
                    .downcast_ref::<sqlx::Error>()
                    .is_some_and(|e| match e {
                        sqlx::Error::Database(db) => db
                            .code()
                            .is_some_and(|code| matches!(code.as_ref(), "5" | "6" | "517" | "261")),
                        sqlx::Error::PoolTimedOut => true,
                        _ => false,
                    });
                if !transient || tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
}

pub(crate) async fn write_output(
    store: Store,
    id: Uuid,
    pid: u32,
    launched: DateTime<Utc>,
    mut events: mpsc::Receiver<EventKind>,
    failed: CancellationToken,
) -> OutputReport {
    let mut failure = retry(|| store.start(id, pid, launched)).await.err();
    if failure.is_some() {
        failed.cancel();
    }
    while let Some(event) = events.recv().await {
        if failure.is_none()
            && let Err(error) = retry(|| store.record(id, event.clone())).await
        {
            failed.cancel();
            failure = Some(error);
        }
        // After storage failure, continue consuming so pipes can drain and the
        // controller can always terminate/reap. Missing output is reported below.
    }
    OutputReport {
        complete: failure.is_none(),
        warning: failure.map(|error| {
            format!("output/state persistence failed; output may be incomplete: {error:#}")
        }),
    }
}
