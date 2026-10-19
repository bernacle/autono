//! Registry-owned results outlive execution and every persistence retry budget.
use std::time::Duration;

use anyhow::Result;
use autono_protocol::{ObservedResult, PersistenceStatus, RunOwnership, RunPhase};
use uuid::Uuid;

use super::{Finalization, RunStatus, Supervisor};

#[derive(Debug, thiserror::Error)]
#[error("persistence is degraded; new executions are disabled until owned results are committed")]
pub struct PersistenceDegraded;

#[derive(Debug, thiserror::Error)]
#[error(
    "shutdown incomplete: {runs:?} still have unpersisted or unresolved results; observed results may be lost if the daemon exits"
)]
pub struct ShutdownIncomplete {
    pub runs: Vec<Uuid>,
}

impl Supervisor {
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
        self.inner.changed.subscribe()
    }

    pub fn ownership(&self, id: Uuid) -> Option<RunOwnership> {
        self.registry().runs.get(&id).map(|control| RunOwnership {
            run_id: id,
            phase: control.phase,
            observed_result: control
                .finalization
                .as_ref()
                .map(|record| record.result.clone()),
            persistence_error: control.persistence_error.clone(),
            failed_attempts: control.failed_attempts,
        })
    }

    pub fn persistence(&self) -> PersistenceStatus {
        let registry = self.registry();
        PersistenceStatus {
            healthy: registry.runs.values().all(|control| {
                control.persistence_error.is_none() && control.phase != RunPhase::RecoveryRequired
            }),
            owned_runs: registry
                .runs
                .iter()
                .map(|(id, control)| RunOwnership {
                    run_id: *id,
                    phase: control.phase,
                    observed_result: control
                        .finalization
                        .as_ref()
                        .map(|record| record.result.clone()),
                    persistence_error: control.persistence_error.clone(),
                    failed_attempts: control.failed_attempts,
                })
                .collect(),
        }
    }

    pub(super) fn persistence_failure(&self, id: Uuid, error: String) {
        if let Some(control) = self.registry().runs.get_mut(&id) {
            control.persistence_error = Some(error);
            control.failed_attempts = control.failed_attempts.saturating_add(1);
        }
        self.inner.changed.send_replace(());
    }

    pub(super) fn recovery_required(&self, id: Uuid, error: String) {
        if let Some(control) = self.registry().runs.get_mut(&id) {
            control.phase = RunPhase::RecoveryRequired;
            control.persistence_error = Some(error.clone());
        }
        tracing::error!(run_id = %id, %error, "owned run requires recovery");
        self.inner.changed.send_replace(());
    }

    pub(super) fn observe_finalization(&self, id: Uuid, record: Finalization) {
        if let Some(control) = self.registry().runs.get_mut(&id) {
            control.phase = RunPhase::Finalizing;
            control.finalization = Some(record);
        }
        self.inner.changed.send_replace(());
    }

    pub(super) fn complete_unstarted(&self, id: Uuid, status: RunStatus, detail: String) {
        self.finalize_owned(
            id,
            Finalization {
                pid: None,
                launched: None,
                result: ObservedResult {
                    status,
                    exit_code: None,
                    detail: Some(detail),
                    cleanup_warnings: Vec::new(),
                    output_complete: Some(true),
                    finished_at: chrono::Utc::now(),
                },
            },
        );
    }

    pub(super) fn finalize_owned(&self, id: Uuid, record: Finalization) {
        self.observe_finalization(id, record.clone());
        let this = self.clone();
        self.inner.finalizers.spawn(async move {
            let mut backoff = Duration::from_millis(100);
            loop {
                if this.inner.abandon.is_cancelled() { return; }
                // Never time out/retry an individual COMMIT on an assumption that
                // cancellation rolled it back. A matching committed result is a
                // no-op, including any later retry after an ambiguous response.
                match this.inner.store.finalize(id, record.pid, record.launched, &record.result).await {
                    Ok(()) => {
                        this.registry().runs.remove(&id); // releases the owned permit
                        this.inner.changed.send_replace(());
                        return;
                    }
                    Err(error) => {
                        this.persistence_failure(id, format!("terminal persistence failed: {error:#}"));
                        tracing::warn!(run_id = %id, %error, "retaining finalization ownership; retrying");
                    }
                }
                tokio::select! {
                    _ = this.inner.abandon.cancelled() => return,
                    _ = tokio::time::sleep(backoff) => {},
                }
                backoff = (backoff * 2).min(Duration::from_secs(2));
            }
        });
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.shutdown_with_timeout(Duration::from_secs(10)).await
    }

    /// Deadline starts when shutdown is signalled. Process cleanup always runs
    /// first; unresolved finalizations remain explicitly owned on failure.
    pub async fn shutdown_with_timeout(&self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        self.signal_shutdown();
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
        self.inner.power.shutdown().await;
        self.inner.finalizers.close();
        let _ = tokio::time::timeout_at(deadline, self.inner.finalizers.wait()).await;
        if self.registry().runs.is_empty() {
            return Ok(());
        }
        // This is an explicit unsuccessful shutdown handoff, never a successful
        // graceful exit. Records/permits remain available for inspection until
        // the owner exits. A hard exit before a terminal commit can lose results.
        self.inner.abandon.cancel();
        let runs: Vec<_> = self.registry().runs.keys().copied().collect();
        for id in &runs {
            self.recovery_required(
                *id,
                "shutdown deadline expired with unpersisted result".into(),
            );
        }
        Err(ShutdownIncomplete { runs }.into())
    }
}
