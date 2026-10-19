#![cfg(unix)]

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use autono_core::{
    EventKind, Run, RunSpec, RunStatus, Store, Supervisor,
    power::{StayAwake, SystemPower},
    supervisor::{PersistenceDegraded, ShutdownIncomplete},
};
use autono_protocol::{RunOwnership, RunPhase, Submission};
use sqlx::Connection;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    runtime: Supervisor,
}
impl Fixture {
    async fn new(slots: usize) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("runtime.sqlite3")).await?;
        let runtime = Supervisor::with_limits(
            store.clone(),
            Arc::new(StayAwake::new(false, Arc::new(SystemPower))),
            slots,
            8,
        );
        Ok(Self {
            dir,
            store,
            runtime,
        })
    }
    async fn connection(&self) -> Result<sqlx::SqliteConnection> {
        Ok(sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(self.dir.path().join("runtime.sqlite3")),
        )
        .await?)
    }
    async fn submit(&self, gate: &str) -> Result<(Run, Submission)> {
        let fifo = self.dir.path().join(gate);
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())?;
        // SAFETY: valid path in the owned temporary directory.
        anyhow::ensure!(
            unsafe { libc::mkfifo(name.as_ptr(), 0o600) } == 0,
            "create FIFO"
        );
        let spec = RunSpec {
            command: vec![
                "sh".into(),
                "-c".into(),
                format!("echo ready; read go < {gate}; printf final-stderr >&2; exit 42"),
            ],
            cwd: self.dir.path().to_owned(),
            env: BTreeMap::new(),
            keep_awake: false,
            timeout_seconds: None,
        };
        let submission = Submission {
            submission_key: Uuid::new_v4(),
            spec,
        };
        let run = self.runtime.submit_keyed(submission.clone()).await?;
        let mut changed = self.store.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self
                    .store
                    .events(run.id, 0, 500)
                    .await?
                    .iter()
                    .any(|event| matches!(event.kind, EventKind::Output { .. }))
                {
                    return Ok::<_, anyhow::Error>(());
                }
                changed.changed().await?;
            }
        })
        .await??;
        Ok((run, submission))
    }
    async fn exit(&self, id: Uuid, gate: &str) -> Result<()> {
        let finished = self
            .runtime
            .control_finished(id)
            .context("owned execution")?;
        let mut pipe = tokio::fs::OpenOptions::new()
            .write(true)
            .open(self.dir.path().join(gate))
            .await?;
        pipe.write_all(b"go\n").await?;
        drop(pipe);
        tokio::time::timeout(Duration::from_secs(3), finished.cancelled()).await?;
        Ok(())
    }
    async fn owned(
        &self,
        id: Uuid,
        condition: impl Fn(&RunOwnership) -> bool,
    ) -> Result<RunOwnership> {
        let mut changed = self.runtime.subscribe();
        tokio::time::timeout(Duration::from_secs(35), async {
            loop {
                let owner = self
                    .runtime
                    .ownership(id)
                    .context("finalization ownership disappeared")?;
                if condition(&owner) {
                    return Ok(owner);
                }
                changed.changed().await?;
            }
        })
        .await?
    }
    async fn committed(&self, ids: &[Uuid]) -> Result<()> {
        let mut changed = self.runtime.subscribe();
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                if ids.iter().all(|id| self.runtime.ownership(*id).is_none()) {
                    return Ok::<_, anyhow::Error>(());
                }
                changed.changed().await?;
            }
        })
        .await??;
        for id in ids {
            let run = self.store.get(*id).await?;
            assert_eq!(run.status, RunStatus::Failed);
            assert_eq!(run.exit_code, Some(42));
            let events = self.store.events(*id, 0, 500).await?;
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event.kind,
                        EventKind::StateChanged {
                            to: RunStatus::Failed,
                            ..
                        }
                    ))
                    .count(),
                1
            );
        }
        Ok(())
    }
}

#[tokio::test]
async fn long_sqlite_lock_keeps_concurrent_results_owned_and_recovers_without_restart() -> Result<()>
{
    let fixture = Fixture::new(3).await?;
    let mut runs = Vec::new();
    for gate in ["first", "second", "third"] {
        runs.push((gate, fixture.submit(gate).await?));
    }
    let mut lock = fixture.connection().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut lock).await?;
    let began = tokio::time::Instant::now();
    for (gate, (run, _)) in &runs {
        fixture.exit(run.id, gate).await?;
    }
    let owner = fixture
        .owned(runs[0].1.0.id, |owner| owner.failed_attempts >= 11)
        .await?;
    // Retry notifications synchronize the test; the lock is deliberately retained
    // longer than both former finalization retry budgets, with no readiness sleeps.
    assert!(began.elapsed() > Duration::from_secs(10));
    assert_eq!(owner.phase, RunPhase::Finalizing);
    let snapshot = fixture.runtime.persistence();
    assert!(!snapshot.healthy);
    assert_eq!(snapshot.owned_runs.len(), 3);
    let capacity = fixture.runtime.admission();
    assert_eq!(
        (
            capacity.active_runs,
            capacity.executing_runs,
            capacity.finalizing_runs
        ),
        (3, 0, 3)
    );
    for (_, (run, submission)) in &runs {
        let owner = fixture
            .runtime
            .ownership(run.id)
            .context("retained owner")?;
        let result = owner.observed_result.context("retained observed result")?;
        assert_eq!(
            (result.status, result.exit_code),
            (RunStatus::Failed, Some(42))
        );
        assert_eq!(result.output_complete, Some(false));
        assert!(!result.cleanup_warnings.is_empty());
        assert_eq!(fixture.store.get(run.id).await?.status, RunStatus::Running);
        assert_eq!(
            fixture.runtime.submit_keyed(submission.clone()).await?.id,
            run.id
        );
        fixture.runtime.cancel(run.id).await?; // Cannot replace an observed exit.
        assert_eq!(
            fixture
                .runtime
                .ownership(run.id)
                .and_then(|owner| owner.observed_result)
                .and_then(|result| result.exit_code),
            Some(42)
        );
    }
    let fresh = fixture
        .runtime
        .submit(runs[0].1.1.spec.clone())
        .await
        .expect_err("degraded runtime must reject fresh execution");
    assert!(fresh.is::<PersistenceDegraded>());
    sqlx::query("ROLLBACK").execute(&mut lock).await?;
    let ids: Vec<_> = runs.iter().map(|(_, (run, _))| run.id).collect();
    fixture.committed(&ids).await?;
    assert!(fixture.runtime.persistence().healthy);
    assert!(fixture.runtime.persistence().owned_runs.is_empty());
    assert_eq!(fixture.runtime.admission().active_runs, 0);
    assert_eq!(fixture.runtime.admission().finalizing_runs, 0);
    // Successful recovery releases all capacity for subsequent process attempts.
    let fresh = fixture
        .runtime
        .submit(RunSpec {
            command: vec!["true".into()],
            ..runs[0].1.1.spec.clone()
        })
        .await?;
    fixture.runtime.shutdown().await?;
    assert!(fixture.store.get(fresh.id).await?.status.is_terminal());
    Ok(())
}

#[tokio::test]
async fn cancellation_and_shutdown_preserve_pending_finalization_until_lock_release() -> Result<()>
{
    let fixture = Fixture::new(1).await?;
    let (run, _) = fixture.submit("shutdown-gate").await?;
    let mut lock = fixture.connection().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut lock).await?;
    fixture.exit(run.id, "shutdown-gate").await?;
    fixture
        .owned(run.id, |owner| owner.failed_attempts > 0)
        .await?;
    fixture.runtime.cancel(run.id).await?;
    let (started, wait) = tokio::sync::oneshot::channel();
    let shutdown = tokio::spawn({
        let runtime = fixture.runtime.clone();
        async move {
            runtime.signal_shutdown();
            let _ = started.send(());
            runtime.shutdown_with_timeout(Duration::from_secs(5)).await
        }
    });
    wait.await?;
    assert_eq!(
        fixture
            .runtime
            .ownership(run.id)
            .and_then(|owner| owner.observed_result)
            .and_then(|result| result.exit_code),
        Some(42)
    );
    sqlx::query("ROLLBACK").execute(&mut lock).await?;
    tokio::time::timeout(Duration::from_secs(6), shutdown).await???;
    fixture.committed(&[run.id]).await?;
    assert_eq!(fixture.runtime.admission().active_runs, 0);
    Ok(())
}

#[tokio::test]
async fn persistent_failure_makes_shutdown_fail_and_retains_an_observable_recovery_record()
-> Result<()> {
    let fixture = Fixture::new(1).await?;
    let (run, _) = fixture.submit("failure-gate").await?;
    let mut connection = fixture.connection().await?;
    sqlx::query("CREATE TRIGGER reject_terminal BEFORE UPDATE ON runs WHEN NEW.status IN ('FAILED','SUCCEEDED','CANCELLED','INTERRUPTED') BEGIN SELECT RAISE(ABORT, 'persistent finalization failure'); END").execute(&mut connection).await?;
    fixture.exit(run.id, "failure-gate").await?;
    fixture
        .owned(run.id, |owner| owner.failed_attempts > 0)
        .await?;
    let error = fixture
        .runtime
        .shutdown_with_timeout(Duration::from_millis(250))
        .await
        .expect_err("unpersisted outcome cannot be a successful shutdown");
    assert!(error.is::<ShutdownIncomplete>());
    let owner = fixture
        .runtime
        .ownership(run.id)
        .context("shutdown must retain recovery ownership")?;
    assert_eq!(owner.phase, RunPhase::RecoveryRequired);
    assert_eq!(
        owner.observed_result.context("observed outcome")?.exit_code,
        Some(42)
    );
    assert_eq!(fixture.runtime.admission().active_runs, 1);
    assert!(!fixture.runtime.persistence().healthy);
    assert_eq!(fixture.store.get(run.id).await?.status, RunStatus::Running);
    assert!(
        !fixture.store.events(run.id, 0, 500).await?.iter().any(
            |event| matches!(event.kind, EventKind::StateChanged { to, .. } if to.is_terminal())
        )
    );
    Ok(())
}
