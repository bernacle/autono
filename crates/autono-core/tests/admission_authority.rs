#![cfg(unix)]
//! Real SQLite admission contention: never drop a commit future to cancel work.
use anyhow::{Context, Result};
use autono_core::{RunStatus, Store, Supervisor, engineering::process::ProcessRunner};
use chrono::{Duration as ChronoDuration, Utc};
use sqlx::Connection;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

async fn eventually(mut condition: impl AsyncFnMut() -> Result<bool>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !condition().await? {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

#[tokio::test]
async fn sqlite_queued_cancellation_expiry_and_shutdown_reconcile_without_launch_or_replay()
-> Result<()> {
    let mut evidence = Vec::new();
    for boundary in ["cancel", "cancel-disconnected", "expiry", "shutdown"] {
        let dir = tempfile::tempdir()?;
        let db = dir.path().join("runtime.sqlite3");
        let store = Store::open(&db).await?;
        let supervisor = Supervisor::new(store.clone(), false);
        let deadline =
            (boundary == "expiry").then(|| Utc::now() + ChronoDuration::milliseconds(350));
        let runner = ProcessRunner::new(supervisor.clone()).with_deadline(deadline);
        let stop = CancellationToken::new();
        let key = Uuid::new_v4();
        let command = vec![
            "/bin/sh".into(),
            "-c".into(),
            "echo launched > launched; exec /bin/sleep 20".into(),
        ];
        let mut lock = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new().filename(&db),
        )
        .await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut lock).await?;
        let start = |runner: ProcessRunner, stop: CancellationToken, command: Vec<String>| {
            let cwd = dir.path().to_owned();
            tokio::spawn(async move { runner.start(command, &cwd, 30, key, &stop).await })
        };
        let first = start(runner.clone(), stop.clone(), command.clone());
        // Active permit is acquired before the blocked create transaction. This
        // handshake proves the submission is inside admission, not an unpolled future.
        eventually(async || Ok(supervisor.admission().active_runs == 1)).await?;
        let second = start(runner.clone(), stop.clone(), command.clone());
        eventually(async || Ok(supervisor.admission().pending_submissions == 2)).await?;
        assert!(store.find_submission(key).await?.is_none());
        match boundary {
            "cancel" | "cancel-disconnected" => stop.cancel(),
            "shutdown" => supervisor.signal_shutdown(),
            _ => {
                tokio::time::sleep(
                    (deadline.context("deadline")? - Utc::now())
                        .to_std()
                        .unwrap_or_default()
                        + Duration::from_millis(20),
                )
                .await
            }
        }
        if boundary == "cancel-disconnected" {
            first.abort();
        }
        sqlx::query("ROLLBACK").execute(&mut lock).await?;
        let run = if boundary == "cancel-disconnected" {
            assert!(first.await.is_err());
            // Abandoning the original waiter cannot abandon its queued authority.
            second.await??
        } else {
            let run = first.await??;
            let replay = second.await?;
            if boundary == "shutdown" {
                assert!(replay.is_err());
            } else {
                assert_eq!(replay?.id, run.id);
            }
            run
        };
        eventually(async || Ok(store.get(run.id).await?.status.is_terminal())).await?;
        let receipt = store.get(run.id).await?;
        assert_eq!(
            receipt.status,
            match boundary {
                "expiry" => RunStatus::Failed,
                "shutdown" => RunStatus::Interrupted,
                _ => RunStatus::Cancelled,
            }
        );
        assert_eq!(receipt.pid, None);
        assert_eq!(receipt.started_at, None);
        assert_eq!(receipt.output_complete, Some(true));
        assert!(!dir.path().join("launched").exists());
        if boundary == "expiry" {
            assert!(
                receipt
                    .detail
                    .as_deref()
                    .unwrap_or_default()
                    .contains("authorization deadline")
            );
        }
        evidence.push(serde_json::json!({"boundary":boundary,"receipt":receipt,"child_marker_created":false,"matching_key_replayed_after_restart":true,"conflicting_key_denied":true}));
        supervisor.shutdown().await?;
        // Reopening and replaying the exact key with fresh authority cannot launch.
        let reopened = Store::open(&db).await?;
        let restarted = Supervisor::new(reopened.clone(), false);
        let replay = ProcessRunner::new(restarted.clone())
            .start(
                command.clone(),
                dir.path(),
                30,
                key,
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!(
            serde_json::to_value(replay)?,
            serde_json::to_value(receipt)?
        );
        let mut conflict = command;
        conflict.push("changed".into());
        assert!(
            ProcessRunner::new(restarted.clone())
                .start(conflict, dir.path(), 30, key, &CancellationToken::new())
                .await
                .is_err()
        );
        assert!(!dir.path().join("launched").exists());
        restarted.shutdown().await?;
    }
    if let Some(path) = std::env::var_os("AUTONO_ADMISSION_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence)?)?;
    }
    Ok(())
}

#[tokio::test]
async fn stopped_before_submission_and_capacity_retry_never_launch() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(&dir.path().join("runtime.sqlite3")).await?;
    let mut config = autono_core::config::Config::default();
    config.runner.max_active_runs = 1;
    config.runner.keep_awake = false;
    let supervisor = Supervisor::configured(store.clone(), &config.runner);
    let runner = ProcessRunner::new(supervisor.clone());
    let command = vec![
        "/bin/sh".into(),
        "-c".into(),
        "echo launched > forbidden".into(),
    ];
    for expired in [false, true] {
        let stop = CancellationToken::new();
        let key = Uuid::new_v4();
        let controlled =
            runner.with_deadline(expired.then(|| Utc::now() - ChronoDuration::seconds(1)));
        if !expired {
            stop.cancel();
        }
        assert!(
            controlled
                .start(command.clone(), dir.path(), 30, key, &stop)
                .await
                .is_err()
        );
        assert!(store.find_submission(key).await?.is_none());
    }
    let blocker = runner
        .start(
            vec!["/bin/sleep".into(), "20".into()],
            dir.path(),
            30,
            Uuid::new_v4(),
            &CancellationToken::new(),
        )
        .await?;
    let stop = CancellationToken::new();
    let key = Uuid::new_v4();
    let controlled = runner.with_deadline(Some(Utc::now() + ChronoDuration::milliseconds(150)));
    assert!(
        controlled
            .start(command, dir.path(), 30, key, &stop)
            .await
            .is_err()
    );
    assert!(store.find_submission(key).await?.is_none());
    assert!(!dir.path().join("forbidden").exists());
    supervisor.cancel(blocker.id).await?;
    supervisor.shutdown().await
}

struct DelayedPower {
    reached: tokio::sync::Notify,
    release: CancellationToken,
}
impl autono_core::power::PowerBackend for DelayedPower {
    fn acquire(&self) -> autono_core::power::Acquisition<'_> {
        Box::pin(async move {
            self.reached.notify_one();
            self.release.cancelled().await;
            anyhow::bail!("deterministic unavailable power fixture")
        })
    }
}

#[tokio::test]
async fn authority_is_checked_again_after_yielding_immediately_before_spawn() -> Result<()> {
    use autono_core::power::StayAwake;
    use std::sync::Arc;
    for expired in [false, true] {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("runtime.sqlite3")).await?;
        let backend = Arc::new(DelayedPower {
            reached: Default::default(),
            release: Default::default(),
        });
        let supervisor = Supervisor::with_power(
            store.clone(),
            Arc::new(StayAwake::new(true, backend.clone())),
        );
        let deadline = expired.then(|| Utc::now() + ChronoDuration::milliseconds(250));
        let runner = ProcessRunner::new(supervisor.clone()).with_deadline(deadline);
        let stop = CancellationToken::new();
        let key = Uuid::new_v4();
        let run = runner
            .start(
                vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    "echo forbidden > launched".into(),
                ],
                dir.path(),
                10,
                key,
                &stop,
            )
            .await?;
        tokio::time::timeout(Duration::from_secs(3), backend.reached.notified()).await?;
        if expired {
            tokio::time::sleep(
                (deadline.context("deadline")? - Utc::now())
                    .to_std()
                    .unwrap_or_default()
                    + Duration::from_millis(20),
            )
            .await;
        } else {
            stop.cancel();
        }
        backend.release.cancel();
        let output = runner.wait(run.id, &stop).await?;
        assert_eq!(output.run.pid, None);
        assert_eq!(
            output.run.status,
            if expired {
                RunStatus::Failed
            } else {
                RunStatus::Cancelled
            }
        );
        assert!(!dir.path().join("launched").exists());
        supervisor.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn running_controls_and_terminal_replay_preserve_observed_outcomes() -> Result<()> {
    for expired in [false, true] {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("runtime.sqlite3")).await?;
        let supervisor = Supervisor::new(store.clone(), false);
        let runner = ProcessRunner::new(supervisor.clone())
            .with_deadline(expired.then(|| Utc::now() + ChronoDuration::milliseconds(350)));
        let stop = CancellationToken::new();
        let key = Uuid::new_v4();
        let run = runner
            .start(
                vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    "echo running; exec /bin/sleep 20".into(),
                ],
                dir.path(),
                30,
                key,
                &stop,
            )
            .await?;
        eventually(async || {
            Ok(!store
                .events(run.id, 0, 500)
                .await?
                .iter()
                .all(|e| !matches!(e.kind, autono_core::EventKind::Output { .. })))
        })
        .await?;
        if !expired {
            stop.cancel();
        }
        let result = runner.wait(run.id, &stop).await?;
        assert_eq!(
            result.run.status,
            if expired {
                RunStatus::Failed
            } else {
                RunStatus::Cancelled
            }
        );
        assert!(result.run.pid.is_some());
        // Completion observed before a later cancellation retains success on replay.
        let key = Uuid::new_v4();
        let fresh = ProcessRunner::new(supervisor.clone());
        let token = CancellationToken::new();
        let result = fresh
            .run(vec!["/usr/bin/true".into()], dir.path(), 10, key, &token)
            .await?;
        assert!(result.succeeded());
        token.cancel();
        let replay = fresh
            .start(
                vec!["/usr/bin/true".into()],
                dir.path(),
                10,
                key,
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!(replay.id, result.run.id);
        assert_eq!(replay.status, RunStatus::Succeeded);
        supervisor.shutdown().await?;
    }
    Ok(())
}
