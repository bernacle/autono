#[path = "finalization.rs"]
mod finalization;
pub use finalization::{PersistenceDegraded, ShutdownIncomplete};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use autono_protocol::{AdmissionStatus, ObservedResult, RunPhase, Submission};
use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

use crate::{
    EventKind, Run, RunSpec, RunStatus, Store,
    config::RunnerConfig,
    model::OutputStream,
    persistence::{Finalization, retry, write_output},
    power::{StayAwake, SystemPower},
    process::Process,
};

/// Execution controls belong to the workflow, not to its admission waiter.
/// The absolute deadline is checked independently of timer task scheduling.
#[derive(Clone, Default)]
pub(crate) struct ExecutionAuthority {
    pub cancel: CancellationToken,
    pub deadline: Option<chrono::DateTime<chrono::Utc>>,
}
impl ExecutionAuthority {
    fn expired(&self) -> bool {
        self.deadline.is_some_and(|d| chrono::Utc::now() >= d)
    }
    pub(crate) async fn deadline_reached(&self) {
        match self.deadline {
            Some(d) => wait_for_deadline(d).await,
            None => std::future::pending().await,
        }
    }
}

pub(crate) async fn wait_for_deadline(deadline: chrono::DateTime<chrono::Utc>) {
    wait_for_deadline_with_clock(deadline, chrono::Utc::now).await;
}

async fn wait_for_deadline_with_clock(
    deadline: chrono::DateTime<chrono::Utc>,
    mut now: impl FnMut() -> chrono::DateTime<chrono::Utc>,
) {
    // A relative timer wake is only a hint about the absolute authority cutoff.
    // Never translate an early wake / wall-clock adjustment into user cancellation.
    while let Ok(remaining) = (deadline - now()).to_std() {
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(remaining).await;
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    #[tokio::test]
    async fn an_early_relative_wake_does_not_signal_absolute_expiry() {
        let deadline = chrono::Utc::now();
        let mut reads = 0;
        wait_for_deadline_with_clock(deadline, || {
            reads += 1;
            if reads < 3 {
                deadline - chrono::Duration::milliseconds(1)
            } else {
                deadline
            }
        })
        .await;
        assert_eq!(
            reads, 3,
            "the absolute cutoff must be rechecked after every wake"
        );
    }
}

struct Registry {
    accepting: bool,
    runs: HashMap<Uuid, Control>,
}
struct Control {
    cancel: CancellationToken,
    finished: CancellationToken,
    phase: RunPhase,
    finalization: Option<Finalization>,
    persistence_error: Option<String>,
    failed_attempts: u64,
    _slot: OwnedSemaphorePermit,
}
#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}
struct Inner {
    store: Store,
    registry: Mutex<Registry>,
    admission_gate: tokio::sync::Mutex<()>,
    stop: CancellationToken,
    tasks: TaskTracker,
    finalizers: TaskTracker,
    abandon: CancellationToken,
    changed: tokio::sync::watch::Sender<()>,
    active: Arc<Semaphore>,
    pending: Arc<Semaphore>,
    limits: (usize, usize),
    power: Arc<StayAwake>,
}

#[derive(Debug, thiserror::Error)]
#[error("run is terminal or no longer owned by this daemon")]
pub struct NotActive;
#[derive(Debug, thiserror::Error)]
#[error("runtime admission capacity exhausted ({0})")]
pub struct Overloaded(pub &'static str);
#[derive(Debug, thiserror::Error)]
#[error("daemon is shutting down")]
pub struct ShuttingDown;

impl Supervisor {
    pub fn new(store: Store, keep_awake: bool) -> Self {
        Self::configured(
            store,
            &RunnerConfig {
                keep_awake,
                max_active_runs: 8,
                max_pending_submissions: 32,
            },
        )
    }
    pub fn configured(store: Store, config: &RunnerConfig) -> Self {
        Self::with_limits(
            store,
            Arc::new(StayAwake::new(config.keep_awake, Arc::new(SystemPower))),
            config.max_active_runs,
            config.max_pending_submissions,
        )
    }
    pub fn with_power(store: Store, power: Arc<StayAwake>) -> Self {
        Self::with_limits(store, power, 8, 32)
    }
    pub fn with_limits(store: Store, power: Arc<StayAwake>, active: usize, pending: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                admission_gate: tokio::sync::Mutex::new(()),
                registry: Mutex::new(Registry {
                    accepting: true,
                    runs: HashMap::new(),
                }),
                stop: CancellationToken::new(),
                tasks: TaskTracker::new(),
                finalizers: TaskTracker::new(),
                abandon: CancellationToken::new(),
                changed: tokio::sync::watch::channel(()).0,
                active: Arc::new(Semaphore::new(active)),
                pending: Arc::new(Semaphore::new(pending)),
                limits: (active, pending),
                power,
            }),
        }
    }
    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn store(&self) -> &Store {
        &self.inner.store
    }
    pub fn power(&self) -> &Arc<StayAwake> {
        &self.inner.power
    }
    pub fn admission(&self) -> AdmissionStatus {
        let registry = self.registry();
        AdmissionStatus {
            active_runs: self.inner.limits.0 - self.inner.active.available_permits(),
            executing_runs: registry
                .runs
                .values()
                .filter(|control| control.phase == RunPhase::Executing)
                .count(),
            finalizing_runs: registry
                .runs
                .values()
                .filter(|control| {
                    matches!(
                        control.phase,
                        RunPhase::Finalizing | RunPhase::RecoveryRequired
                    )
                })
                .count(),
            pending_submissions: self.inner.limits.1 - self.inner.pending.available_permits(),
            max_active_runs: self.inner.limits.0,
            max_pending_submissions: self.inner.limits.1,
        }
    }
    pub async fn submit(&self, spec: RunSpec) -> Result<Run> {
        self.submit_keyed(Submission {
            submission_key: Uuid::new_v4(),
            spec,
        })
        .await
    }
    pub async fn submit_keyed(&self, submission: Submission) -> Result<Run> {
        self.submit_authorized(submission, ExecutionAuthority::default())
            .await
    }
    pub(crate) async fn submit_authorized(
        &self,
        submission: Submission,
        authority: ExecutionAuthority,
    ) -> Result<Run> {
        submission.spec.validate()?;
        let pending = self
            .inner
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| Overloaded("pending submissions"))?;
        let (reply, response) = oneshot::channel();
        {
            let registry = self.registry();
            anyhow::ensure!(registry.accepting, ShuttingDown);
            let this = self.clone();
            // No await between registering daemon-owned work and releasing the
            // gate shared with shutdown. Dropping HTTP/oneshot cannot cancel it.
            self.inner.tasks.spawn(async move {
                let _pending = pending;
                let result = this.accept(submission, authority).await;
                let _ = reply.send(result);
            });
        }
        response.await.context("daemon admission task stopped")?
    }
    async fn accept(&self, submission: Submission, authority: ExecutionAuthority) -> Result<Run> {
        let _gate = tokio::select! {
            biased;
            _ = self.inner.stop.cancelled() => return Err(ShuttingDown.into()),
            gate = self.inner.admission_gate.lock() => gate,
        };
        if let Some(run) =
            retry(|| self.inner.store.find_submission(submission.submission_key)).await?
        {
            anyhow::ensure!(
                run.spec == submission.spec,
                crate::store::SubmissionConflict
            );
            return Ok(run);
        }
        anyhow::ensure!(!self.inner.stop.is_cancelled(), ShuttingDown);
        anyhow::ensure!(self.persistence().healthy, PersistenceDegraded);
        let slot = self
            .inner
            .active
            .clone()
            .try_acquire_owned()
            .map_err(|_| Overloaded("active runs"))?;
        // Do not cancel an in-flight SQLite mutation: its commit may still finish.
        // Retain identity through reconciliation, then execute observes authority
        // before launch and persists an unstarted terminal receipt when stopped.
        let (run, created) = retry(|| {
            self.inner
                .store
                .create_keyed(submission.submission_key, submission.spec.clone())
        })
        .await?;
        if !created {
            return Ok(run);
        }
        let cancel = CancellationToken::new();
        let finished = CancellationToken::new();
        self.registry().runs.insert(
            run.id,
            Control {
                cancel: cancel.clone(),
                finished: finished.clone(),
                phase: RunPhase::Starting,
                finalization: None,
                persistence_error: None,
                failed_attempts: 0,
                _slot: slot,
            },
        );
        let this = self.clone();
        let owned = run.clone();
        self.inner.tasks.spawn(async move {
            if let Err(error) = this
                .execute(&owned, cancel, finished.clone(), authority)
                .await
            {
                this.recovery_required(owned.id, format!("process controller failed: {error:#}"));
            }
            finished.cancel();
            this.inner.changed.send_replace(());
        });
        Ok(run)
    }
    pub async fn cancel(&self, id: Uuid) -> Result<()> {
        if let Some(cancel) = self.registry().runs.get(&id) {
            cancel.cancel.cancel();
            return Ok(());
        }
        // Only unowned IDs need a database lookup; active cancellation never does.
        self.inner.store.get(id).await?;
        Err(NotActive.into())
    }
    /// Signals completion of process control/reaping attempts, independent of storage finalization.
    pub fn control_finished(&self, id: Uuid) -> Option<CancellationToken> {
        self.registry()
            .runs
            .get(&id)
            .map(|control| control.finished.clone())
    }
    pub fn signal_shutdown(&self) {
        self.registry().accepting = false;
        self.inner.stop.cancel();
    }
    /// A single observation determines both status and reason when signals race.
    /// Finalization remains daemon-owned and does not wait on SQLite here.
    fn finish_if_stopped(
        &self,
        id: Uuid,
        cancel: &CancellationToken,
        authority: &ExecutionAuthority,
    ) -> bool {
        let shutdown = self.inner.stop.is_cancelled();
        let expired = authority.expired();
        if shutdown || expired || cancel.is_cancelled() || authority.cancel.is_cancelled() {
            let (status, reason) = if shutdown {
                (RunStatus::Interrupted, "daemon stopped before launch")
            } else if expired {
                (
                    RunStatus::Failed,
                    "authorization deadline exceeded before launch",
                )
            } else {
                (RunStatus::Cancelled, "stopped before launch")
            };
            self.complete_unstarted(id, status, reason.into());
            true
        } else {
            false
        }
    }
    async fn execute(
        &self,
        run: &Run,
        cancel: CancellationToken,
        finished: CancellationToken,
        authority: ExecutionAuthority,
    ) -> Result<()> {
        if self.finish_if_stopped(run.id, &cancel, &authority) {
            return Ok(());
        }
        let mut warnings = Vec::new();
        if run.spec.keep_awake
            && let Some(warning) = self.inner.power.enter().await
        {
            warnings.push(warning);
        }
        let result = self
            .execute_process(run, cancel, warnings, finished, authority)
            .await;
        if run.spec.keep_awake {
            self.inner.power.leave().await;
        }
        result
    }
    async fn execute_process(
        &self,
        run: &Run,
        cancel: CancellationToken,
        mut warnings: Vec<String>,
        control_finished: CancellationToken,
        authority: ExecutionAuthority,
    ) -> Result<()> {
        let mut command = Command::new(&run.spec.command[0]);
        command
            .args(&run.spec.command[1..])
            .current_dir(&run.spec.cwd)
            .envs(&run.spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Power acquisition can yield. Honor a stop received during it before
        // launching anything; persistence of the stop may follow asynchronously.
        if self.finish_if_stopped(run.id, &cancel, &authority) {
            return Ok(());
        }
        let launched = chrono::Utc::now();
        let clock = tokio::time::Instant::now();
        let mut process = match Process::spawn(&mut command) {
            Ok(process) => process,
            Err(error) => {
                self.complete_unstarted(
                    run.id,
                    RunStatus::Failed,
                    format!("spawn failed: {error:#}"),
                );
                return Ok(());
            }
        };
        if let Some(control) = self.registry().runs.get_mut(&run.id) {
            control.phase = RunPhase::Executing;
        }
        self.inner.changed.send_replace(());
        let (events, receiver) = mpsc::channel(32);
        let failed = CancellationToken::new();
        let writer = tokio::spawn(write_output(
            self.inner.store.clone(),
            run.id,
            process.pid,
            launched,
            receiver,
            failed.clone(),
        ));
        for message in warnings.drain(..) {
            let _ = events.try_send(EventKind::Warning { message });
        }
        let mut readers = tokio::task::JoinSet::new();
        if let Some(stdout) = process.child.stdout.take() {
            readers.spawn(read_output(stdout, OutputStream::Stdout, events.clone()));
        }
        if let Some(stderr) = process.child.stderr.take() {
            readers.spawn(read_output(stderr, OutputStream::Stderr, events.clone()));
        }
        let mut power_errors = self.inner.power.subscribe();
        let deadline = clock + Duration::from_secs(run.spec.timeout_seconds.unwrap_or(31_536_000));
        // This loop performs NO database work or bounded-channel sends. Output
        // backpressure cannot delay cancellation, shutdown, or the launch deadline.
        let observed = loop {
            tokio::select! {
                biased;
                status = process.exited() => break status.map(Some),
                _ = self.inner.stop.cancelled() => break Ok(None),
                _ = cancel.cancelled() => break Ok(None),
                _ = authority.cancel.cancelled() => break Ok(None),
                _ = authority.deadline_reached() => break Ok(None),
                _ = tokio::time::sleep_until(deadline), if run.spec.timeout_seconds.is_some() => break Ok(None),
                _ = failed.cancelled() => break Ok(None),
                result = power_errors.changed(), if run.spec.keep_awake => {
                    if result.is_ok() && let Some(message) = power_errors.borrow().clone() && events.try_send(EventKind::Warning { message: message.clone() }).is_err() { warnings.push(message); }
                }
            }
        };
        let (status, mut exit_code, detail) = match observed {
            Ok(Some(status)) => (
                if status.success() {
                    RunStatus::Succeeded
                } else {
                    RunStatus::Failed
                },
                status.code(),
                (!status.success()).then(|| format!("process exited: {status}")),
            ),
            Ok(None) if self.inner.stop.is_cancelled() => (
                RunStatus::Interrupted,
                None,
                Some("daemon shut down".into()),
            ),
            Ok(None) if authority.expired() => (
                RunStatus::Failed,
                None,
                Some("authorization deadline exceeded".into()),
            ),
            Ok(None) if cancel.is_cancelled() || authority.cancel.is_cancelled() => {
                (RunStatus::Cancelled, None, Some("cancelled by user".into()))
            }
            Ok(None)
                if run.spec.timeout_seconds.is_some()
                    && tokio::time::Instant::now() >= deadline =>
            {
                (RunStatus::Failed, None, Some("timeout exceeded".into()))
            }
            Ok(None) => (
                RunStatus::Interrupted,
                None,
                Some("output persistence failed".into()),
            ),
            Err(error) => (
                RunStatus::Interrupted,
                None,
                Some(format!("process observation failed: {error:#}")),
            ),
        };
        let finished_at = chrono::Utc::now();
        let mut finalization = Finalization {
            pid: Some(process.pid),
            launched: Some(launched),
            result: ObservedResult {
                status,
                exit_code,
                detail,
                cleanup_warnings: Vec::new(),
                output_complete: None,
                finished_at,
            },
        };
        self.observe_finalization(run.id, finalization.clone());
        if let Err(error) = process.terminate() {
            warnings.push(format!("process-group cleanup uncertain: {error:#}"));
            if let Err(error) = process.kill_direct() {
                warnings.push(format!("direct-child cleanup failed: {error:#}"));
            }
        }
        match tokio::time::timeout(Duration::from_secs(3), process.reap()).await {
            Ok(Ok(reaped)) => {
                if exit_code.is_none() {
                    exit_code = reaped.code();
                }
            }
            Ok(Err(error)) => warnings.push(format!("child reaping failed: {error:#}")),
            Err(_) => warnings
                .push("child reaping timed out; OS termination could not be confirmed".into()),
        }
        finalization.result.exit_code = exit_code;
        finalization.result.cleanup_warnings = warnings.clone();
        self.observe_finalization(run.id, finalization.clone());
        control_finished.cancel();
        // Reaping/group cleanup completes before any persistence wait. Draining is
        // bounded even when a detached descendant retains the pipe or disk stalls.
        let mut output_complete = true;
        let drained = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(result) = readers.join_next().await {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        output_complete = false;
                        warnings.push(format!("output read failed: {error:#}"));
                    }
                    Err(error) => {
                        output_complete = false;
                        warnings.push(format!("output reader failed: {error}"));
                    }
                }
            }
        })
        .await;
        if drained.is_err() {
            output_complete = false;
            warnings.push("output drain timed out; output may be incomplete".into());
            readers.abort_all();
        }
        drop(readers);
        drop(events);
        finalization.result.cleanup_warnings = warnings;
        finalization.result.output_complete = Some(output_complete);
        self.observe_finalization(run.id, finalization.clone());
        // Only the output writer has a finite wait budget. The observed terminal
        // result has already transferred to registry ownership and is never dropped.
        let mut writer = writer;
        match tokio::time::timeout(Duration::from_secs(10), &mut writer).await {
            Ok(Ok(report)) => {
                if !report.complete {
                    finalization.result.output_complete = Some(false);
                }
                if let Some(warning) = report.warning {
                    self.persistence_failure(run.id, warning.clone());
                    finalization.result.cleanup_warnings.push(warning);
                }
            }
            result => {
                writer.abort();
                let message = match result {
                    Ok(Err(error)) => format!("output writer failed: {error}"),
                    _ => {
                        let _ = writer.await;
                        "output writer drain timed out".into()
                    }
                };
                finalization.result.output_complete = Some(false);
                finalization.result.cleanup_warnings.push(message.clone());
                self.persistence_failure(run.id, message);
            }
        }
        self.finalize_owned(run.id, finalization);
        Ok(())
    }
}

async fn read_output(
    mut reader: impl AsyncRead + Unpin,
    stream: OutputStream,
    events: mpsc::Sender<EventKind>,
) -> Result<()> {
    loop {
        let mut bytes = vec![0; 8192];
        let count = reader.read(&mut bytes).await?;
        if count == 0 {
            return Ok(());
        }
        bytes.truncate(count);
        if events
            .send(EventKind::Output {
                stream,
                data: STANDARD.encode(bytes),
            })
            .await
            .is_err()
        {
            return Ok(());
        }
    }
}
