use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use autono_core::{
    EventKind, Run, RunSpec, RunStatus, Store, Supervisor,
    model::OutputStream,
    power::{Acquisition, Inhibition, PowerBackend, PowerLease, StayAwake},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use tempfile::TempDir;

fn spec(command: &[&str]) -> RunSpec {
    RunSpec {
        command: command.iter().map(|s| s.to_string()).collect(),
        cwd: std::env::current_dir().expect("cwd"),
        env: BTreeMap::new(),
        timeout_seconds: None,
        keep_awake: false,
    }
}

async fn fixture() -> Result<(TempDir, Store, Supervisor)> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(&dir.path().join("runtime.sqlite3")).await?;
    let supervisor = Supervisor::new(store.clone(), false);
    Ok((dir, store, supervisor))
}

async fn terminal(store: &Store, id: uuid::Uuid) -> Result<Run> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut changed = store.subscribe();
        loop {
            let run = store.get(id).await?;
            if run.status.is_terminal() {
                return Ok(run);
            }
            changed.changed().await?;
        }
    })
    .await
    .context("run did not finish")?
}

#[cfg(unix)]
async fn ready(store: &Store, id: uuid::Uuid) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut changed = store.subscribe();
        loop {
            if store
                .events(id, 0, 500)
                .await?
                .iter()
                .any(|event| matches!(&event.kind, EventKind::Output { .. }))
            {
                return Ok(());
            }
            anyhow::ensure!(
                !store.get(id).await?.status.is_terminal(),
                "run ended before ready"
            );
            changed.changed().await?;
        }
    })
    .await
    .context("run did not become ready")?
}

#[tokio::test]
async fn persistence_transitions_events_and_reconciliation_survive_reopen() -> Result<()> {
    let (dir, store, supervisor) = fixture().await?;
    let pending = store.create(spec(&["unused"])).await?;
    let running = store.create(spec(&["unused"])).await?;
    store
        .transition(running.id, RunStatus::Running, Some(999_999), None, None)
        .await?;
    let failed = store.create(spec(&["unused"])).await?;
    store
        .transition(
            failed.id,
            RunStatus::Failed,
            None,
            None,
            Some("spawn failed".into()),
        )
        .await?;
    assert!(
        store
            .transition(failed.id, RunStatus::Running, None, None, None)
            .await
            .is_err()
    );
    assert!(
        store
            .transition(pending.id, RunStatus::Succeeded, None, None, None)
            .await
            .is_err()
    );
    assert_eq!(store.events(pending.id, 0, 500).await?.len(), 1);
    supervisor.shutdown().await?;
    drop(supervisor);
    drop(store);
    let reopened = Store::open(&dir.path().join("runtime.sqlite3")).await?;
    assert_eq!(reopened.reconcile().await?, 2);
    assert_eq!(reopened.reconcile().await?, 0);
    for id in [pending.id, running.id] {
        let run = reopened.get(id).await?;
        assert_eq!(run.status, RunStatus::Interrupted);
        assert!(run.finished_at.is_some());
        assert!(
            run.detail
                .as_deref()
                .is_some_and(|s| s.contains("ownership"))
        );
        let events = reopened.events(id, 0, 500).await?;
        assert!(matches!(
            events.last().map(|e| &e.kind),
            Some(EventKind::StateChanged {
                to: RunStatus::Interrupted,
                ..
            })
        ));
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        let cursor = events[0].sequence;
        assert_eq!(
            reopened.events(id, cursor, 500).await?.len(),
            events.len() - 1
        );
    }
    assert_eq!(reopened.get(failed.id).await?.status, RunStatus::Failed);
    reopened.health().await?;
    Ok(())
}

#[tokio::test]
async fn failed_spawn_is_durable_and_never_running() -> Result<()> {
    let (_dir, store, supervisor) = fixture().await?;
    let run = supervisor
        .submit(spec(&["/autono/nonexistent/executable"]))
        .await?;
    let run = terminal(&store, run.id).await?;
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.started_at.is_none());
    assert!(
        run.detail
            .as_deref()
            .is_some_and(|s| s.contains("spawn failed"))
    );
    supervisor.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn append_only_history_and_transaction_rollback_protect_lifecycle_consistency() -> Result<()>
{
    use sqlx::Connection;
    let (dir, store, supervisor) = fixture().await?;
    let run = store.create(spec(&["unused"])).await?;
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("runtime.sqlite3")),
    )
    .await?;
    assert!(
        sqlx::query("DELETE FROM events")
            .execute(&mut connection)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE events SET payload = payload")
            .execute(&mut connection)
            .await
            .is_err()
    );
    sqlx::query("CREATE TRIGGER reject_new_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'injected event failure'); END").execute(&mut connection).await?;
    assert!(
        store
            .transition(run.id, RunStatus::Running, Some(1), None, None)
            .await
            .is_err()
    );
    assert_eq!(store.get(run.id).await?.status, RunStatus::Pending);
    assert_eq!(store.events(run.id, 0, 500).await?.len(), 1);
    sqlx::query("DROP TRIGGER reject_new_event")
        .execute(&mut connection)
        .await?;
    store
        .transition(run.id, RunStatus::Running, Some(1), None, None)
        .await?;
    assert_eq!(store.events(run.id, 0, 500).await?.len(), 2);
    supervisor.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn output_persistence_failure_stops_execution_and_records_interruption() -> Result<()> {
    use sqlx::Connection;
    let (dir, store, supervisor) = fixture().await?;
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("runtime.sqlite3")),
    )
    .await?;
    sqlx::query("CREATE TRIGGER reject_output BEFORE INSERT ON events WHEN json_extract(NEW.payload, '$.type') = 'output' BEGIN SELECT RAISE(ABORT, 'injected output failure'); END").execute(&mut connection).await?;
    let run = supervisor
        .submit(spec(&["sh", "-c", "echo cannot-persist; exec sleep 300"]))
        .await?;
    let finished = terminal(&store, run.id).await?;
    assert_eq!(finished.status, RunStatus::Interrupted);
    assert!(
        finished
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("persistence failed"))
    );
    assert!(store.events(run.id, 0, 500).await?.len() >= 3);
    supervisor.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn leader_exit_also_cleans_up_descendants_holding_output_pipes() -> Result<()> {
    let (_dir, store, supervisor) = fixture().await?;
    let run = supervisor
        .submit(spec(&["sh", "-c", "sleep 300 & echo done; exit 0"]))
        .await?;
    let finished = terminal(&store, run.id).await?;
    assert_eq!(finished.status, RunStatus::Succeeded);
    assert!(
        !store
            .events(run.id, 0, 500)
            .await?
            .iter()
            .any(|event| matches!(event.kind, EventKind::Warning { .. }))
    );
    supervisor.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn success_preserves_both_streams_binary_output_cwd_env_and_closed_stdin() -> Result<()> {
    let (dir, store, supervisor) = fixture().await?;
    let mut input = spec(&[
        "sh",
        "-c",
        "printf '%s' \"$AUTONO_TEST\"; printf '\\377\\000' >&2; cat; pwd",
    ]);
    input.cwd = dir.path().to_owned();
    input.env.insert("AUTONO_TEST".into(), "hello".into());
    let run = supervisor.submit(input).await?;
    assert_eq!(terminal(&store, run.id).await?.status, RunStatus::Succeeded);
    let events = store.events(run.id, 0, 500).await?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    for event in &events {
        if let EventKind::Output { stream, data } = &event.kind {
            let destination = match stream {
                OutputStream::Stdout => &mut stdout,
                OutputStream::Stderr => &mut stderr,
            };
            destination.extend(STANDARD.decode(data)?);
        }
    }
    let cwd = tokio::fs::canonicalize(dir.path()).await?;
    assert_eq!(
        String::from_utf8(stdout)?,
        format!("hello{}\n", cwd.display())
    );
    assert_eq!(stderr, [255, 0]);
    assert!(matches!(
        events.last().map(|e| &e.kind),
        Some(EventKind::StateChanged {
            to: RunStatus::Succeeded,
            ..
        })
    ));
    supervisor.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn failure_retains_exit_code_and_stderr() -> Result<()> {
    let (_dir, store, supervisor) = fixture().await?;
    let run = supervisor
        .submit(spec(&["sh", "-c", "echo broken >&2; exit 42"]))
        .await?;
    let finished = terminal(&store, run.id).await?;
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(finished.exit_code, Some(42));
    assert!(store.events(run.id, 0, 500).await?.iter().any(|e| matches!(
        &e.kind,
        EventKind::Output {
            stream: OutputStream::Stderr,
            ..
        }
    )));
    supervisor.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_kills_owned_group_and_reaps_child() -> Result<()> {
    let (dir, store, supervisor) = fixture().await?;
    // Child keeps its own PID in a file, and the leader waits. Readiness is an output event.
    let mut input = spec(&[
        "sh",
        "-c",
        "sleep 300 & echo $! > child.pid; echo ready; wait",
    ]);
    input.cwd = dir.path().to_owned();
    let run = supervisor.submit(input).await?;
    ready(&store, run.id).await?;
    let child_pid: i32 = tokio::fs::read_to_string(dir.path().join("child.pid"))
        .await?
        .trim()
        .parse()?;
    supervisor.cancel(run.id).await?;
    assert_eq!(terminal(&store, run.id).await?.status, RunStatus::Cancelled);
    assert!(supervisor.cancel(run.id).await.is_err());
    // On Linux a killed orphan can remain a zombie until PID 1 reaps it. It must not
    // continue executing. On macOS, kqueue/kill eventually reports it absent.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            // SAFETY: signal 0 only tests process existence; no signal is delivered.
            if unsafe { libc::kill(child_pid, 0) } != 0 {
                break;
            }
            #[cfg(target_os = "linux")]
            if let Ok(status) = tokio::fs::read_to_string(format!("/proc/{child_pid}/stat")).await
                && status.split_whitespace().nth(2) == Some("Z")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    supervisor.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_and_daemon_shutdown_have_distinct_durable_outcomes() -> Result<()> {
    let (_dir, store, supervisor) = fixture().await?;
    let mut input = spec(&["sh", "-c", "echo ready; exec sleep 300"]);
    input.timeout_seconds = Some(1);
    let timeout = supervisor.submit(input).await?;
    let running = supervisor
        .submit(spec(&["sh", "-c", "echo ready; exec sleep 300"]))
        .await?;
    ready(&store, running.id).await?;
    let finished = terminal(&store, timeout.id).await?;
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(finished.detail.as_deref(), Some("timeout exceeded"));
    supervisor.shutdown().await?;
    assert_eq!(store.get(running.id).await?.status, RunStatus::Interrupted);
    assert!(supervisor.submit(spec(&["true"])).await.is_err());
    Ok(())
}

#[derive(Default)]
struct FakePower {
    acquired: AtomicUsize,
    released: Arc<AtomicUsize>,
    fail: bool,
}
struct FakeHandle(Arc<AtomicUsize>);
impl PowerLease for FakeHandle {
    fn check(&self) -> Result<()> {
        Ok(())
    }
}
impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl PowerBackend for FakePower {
    fn acquire(&self) -> Acquisition<'_> {
        Box::pin(async {
            self.acquired.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(!self.fail, "test acquisition failure");
            Ok(Box::new(FakeHandle(self.released.clone())) as Inhibition)
        })
    }
}

#[tokio::test]
async fn inhibition_reference_counts_and_reports_nonfatal_acquisition_failures() -> Result<()> {
    let backend = Arc::new(FakePower::default());
    let power = StayAwake::new(true, backend.clone());
    assert!(power.enter().await.is_none());
    assert!(power.enter().await.is_none());
    assert_eq!(backend.acquired.load(Ordering::SeqCst), 1);
    assert_eq!(power.status().await.eligible_runs, 2);
    power.leave().await;
    assert!(power.status().await.held);
    assert_eq!(backend.released.load(Ordering::SeqCst), 0);
    power.leave().await;
    assert!(!power.status().await.held);
    assert_eq!(backend.released.load(Ordering::SeqCst), 1);
    let failing = StayAwake::new(
        true,
        Arc::new(FakePower {
            fail: true,
            ..FakePower::default()
        }),
    );
    assert!(failing.enter().await.is_some());
    assert!(!failing.status().await.held);
    assert_eq!(failing.status().await.eligible_runs, 1);
    failing.leave().await;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn multiple_runs_share_inhibition_until_shutdown_and_failures_are_events() -> Result<()> {
    let (dir, store, _unused) = fixture().await?;
    let backend = Arc::new(FakePower::default());
    let power = Arc::new(StayAwake::new(true, backend.clone()));
    let supervisor = Supervisor::with_power(store.clone(), power.clone());
    let mut input = spec(&["sh", "-c", "echo ready; exec sleep 300"]);
    input.keep_awake = true;
    let first = supervisor.submit(input.clone()).await?;
    let second = supervisor.submit(input.clone()).await?;
    ready(&store, first.id).await?;
    ready(&store, second.id).await?;
    assert_eq!(power.status().await.eligible_runs, 2);
    assert_eq!(backend.acquired.load(Ordering::SeqCst), 1);
    supervisor.cancel(first.id).await?;
    terminal(&store, first.id).await?;
    assert!(power.status().await.held);
    supervisor.shutdown().await?;
    assert_eq!(backend.released.load(Ordering::SeqCst), 1);
    assert_eq!(power.status().await.eligible_runs, 0);
    let failing = Supervisor::with_power(
        store.clone(),
        Arc::new(StayAwake::new(
            true,
            Arc::new(FakePower {
                fail: true,
                ..FakePower::default()
            }),
        )),
    );
    input.command = vec!["true".into()];
    let run = failing.submit(input).await?;
    assert_eq!(terminal(&store, run.id).await?.status, RunStatus::Succeeded);
    assert!(
        store
            .events(run.id, 0, 500)
            .await?
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Warning { .. }))
    );
    failing.shutdown().await?;
    drop(dir);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_and_cancel_remain_responsive_while_sqlite_writer_is_locked() -> Result<()> {
    use sqlx::Connection;
    use tokio::io::AsyncWriteExt;
    for action in ["timeout", "cancel", "shutdown"] {
        let timeout = action == "timeout";
        let (dir, store, supervisor) = fixture().await?;
        let fifo = dir.path().join("gate");
        let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())?;
        // SAFETY: path is a valid, terminated path in the test's temporary directory.
        anyhow::ensure!(
            unsafe { libc::mkfifo(path.as_ptr(), 0o600) } == 0,
            "mkfifo failed"
        );
        let mut input = spec(&[
            "sh",
            "-c",
            "echo ready; read go < gate; printf blocked-writer; sleep 2; touch forbidden-side-effect",
        ]);
        input.cwd = dir.path().to_owned();
        input.timeout_seconds = timeout.then_some(1);
        let run = supervisor.submit(input).await?;
        ready(&store, run.id).await?;
        let pid = store.get(run.id).await?.pid.context("launched PID")?;
        let control = supervisor
            .control_finished(run.id)
            .context("owned control")?;
        let mut blocker = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("runtime.sqlite3")),
        )
        .await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut blocker).await?;
        let mut gate = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&fifo)
            .await?;
        gate.write_all(b"go\n").await?;
        drop(gate);
        if action == "cancel" {
            tokio::time::timeout(Duration::from_millis(200), supervisor.cancel(run.id)).await??;
        }
        if action == "shutdown" {
            supervisor.signal_shutdown();
        }
        tokio::time::timeout(Duration::from_millis(1800), control.cancelled())
            .await
            .context("storage lock delayed process control")?;
        // SAFETY: signal zero only inspects existence. Cleanup has reaped this owned child.
        assert_ne!(unsafe { libc::kill(i32::try_from(pid)?, 0) }, 0);
        assert!(!tokio::fs::try_exists(dir.path().join("forbidden-side-effect")).await?);
        // The test still holds the write lock when process control finishes.
        sqlx::query("ROLLBACK").execute(&mut blocker).await?;
        let result = terminal(&store, run.id).await?;
        assert_eq!(
            result.status,
            if timeout {
                RunStatus::Failed
            } else if action == "shutdown" {
                RunStatus::Interrupted
            } else {
                RunStatus::Cancelled
            }
        );
        if timeout {
            assert_eq!(result.detail.as_deref(), Some("timeout exceeded"));
        }
        supervisor.shutdown().await?;
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_submission_waiter_cannot_drop_daemon_ownership_or_duplicate_execution()
-> Result<()> {
    use autono_protocol::Submission;
    use sqlx::Connection;
    let (dir, store, supervisor) = fixture().await?;
    let mut input = spec(&["sh", "-c", "echo once >> effects; printf complete"]);
    input.cwd = dir.path().to_owned();
    let submission = Submission {
        submission_key: uuid::Uuid::new_v4(),
        spec: input,
    };
    let mut blocker = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("runtime.sqlite3")),
    )
    .await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut blocker).await?;
    let request = tokio::spawn({
        let runtime = supervisor.clone();
        let submission = submission.clone();
        async move { runtime.submit_keyed(submission).await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while supervisor.admission().active_runs != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    request.abort();
    let _ = request.await;
    assert_eq!(supervisor.admission().pending_submissions, 1);
    sqlx::query("ROLLBACK").execute(&mut blocker).await?;
    let mut changed = store.subscribe();
    let run = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(run) = store.find_submission(submission.submission_key).await? {
                break Ok::<_, anyhow::Error>(run);
            }
            changed.changed().await?;
        }
    })
    .await??;
    assert_eq!(terminal(&store, run.id).await?.status, RunStatus::Succeeded);
    for _ in 0..3 {
        assert_eq!(
            supervisor.submit_keyed(submission.clone()).await?.id,
            run.id
        );
    }
    let mut conflict = submission;
    conflict.spec.command = vec!["false".into()];
    assert!(supervisor.submit_keyed(conflict).await.is_err());
    supervisor.shutdown().await?;
    assert_eq!(
        tokio::fs::read_to_string(dir.path().join("effects")).await?,
        "once\n"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn repeated_rapid_nonzero_exits_preserve_status_stderr_and_cleanup_separation() -> Result<()>
{
    let (_dir, store, supervisor) = fixture().await?;
    for _ in 0..80 {
        let run = supervisor
            .submit(spec(&["sh", "-c", "printf important-error >&2; exit 42"]))
            .await?;
        let finished = terminal(&store, run.id).await?;
        assert_eq!(finished.status, RunStatus::Failed);
        assert_eq!(finished.exit_code, Some(42));
        let mut stderr = Vec::new();
        for event in store.events(run.id, 0, 500).await? {
            if let EventKind::Output {
                stream: OutputStream::Stderr,
                data,
            } = event.kind
            {
                stderr.extend(STANDARD.decode(data)?);
            }
        }
        assert_eq!(stderr, b"important-error");
    }
    supervisor.shutdown().await?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn macos_helper_loss_is_persisted_and_reacquired_without_losing_references() -> Result<()> {
    let (_dir, store, _unused) = fixture().await?;
    let supervisor = Supervisor::new(store.clone(), true);
    let mut input = spec(&["sh", "-c", "echo ready; exec sleep 300"]);
    input.keep_awake = true;
    let first = supervisor.submit(input.clone()).await?;
    let second = supervisor.submit(input).await?;
    ready(&store, first.id).await?;
    ready(&store, second.id).await?;
    let power = supervisor.power().status().await;
    assert_eq!(power.eligible_runs, 2);
    let original = power.helper_pid.context("caffeinate helper")?;
    // SAFETY: target is the freshly acquired helper owned by this test runtime.
    anyhow::ensure!(
        unsafe { libc::kill(i32::try_from(original)?, libc::SIGKILL) } == 0,
        "kill helper"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut changed = store.subscribe();
        loop {
            let events = store.events(first.id, 0, 500).await?;
            if events.iter().any(|event| matches!(&event.kind, EventKind::Warning { message } if message.contains("restored"))) { break Ok::<_, anyhow::Error>(()); }
            changed.changed().await?;
        }
    }).await??;
    let power = supervisor.power().status().await;
    assert_eq!(power.eligible_runs, 2);
    assert!(power.held);
    let replacement = power.helper_pid.context("replacement helper")?;
    assert_ne!(original, replacement);
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut poll = tokio::time::interval(Duration::from_millis(20));
        loop {
            poll.tick().await;
            let assertions = tokio::time::timeout(
                Duration::from_secs(1),
                tokio::process::Command::new("/usr/bin/pmset")
                    .args(["-g", "assertions"])
                    .output(),
            )
            .await??;
            let assertions = String::from_utf8(assertions.stdout)?;
            let own: Vec<_> = assertions
                .lines()
                .filter(|line| line.contains(&format!("pid {replacement}(caffeinate)")))
                .collect();
            if !own.is_empty() {
                assert!(
                    own.iter()
                        .all(|line| line.contains("PreventUserIdleSystemSleep"))
                );
                break Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await??;
    supervisor.cancel(first.id).await?;
    terminal(&store, first.id).await?;
    assert!(supervisor.power().status().await.held);
    supervisor.shutdown().await?;
    assert_eq!(supervisor.power().status().await.eligible_runs, 0);
    assert!(!supervisor.power().status().await.held);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn power_reacquisition_has_bounded_backoff_and_stops_after_last_reference() -> Result<()> {
    let backend = Arc::new(FakePower {
        fail: true,
        ..FakePower::default()
    });
    let power = StayAwake::new(true, backend.clone());
    assert!(power.enter().await.is_some());
    tokio::task::yield_now().await;
    for delay in [100, 200, 400, 800, 1600, 2000] {
        let attempts = backend.acquired.load(Ordering::SeqCst);
        tokio::time::advance(Duration::from_millis(delay - 1)).await;
        tokio::task::yield_now().await;
        assert_eq!(backend.acquired.load(Ordering::SeqCst), attempts);
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(backend.acquired.load(Ordering::SeqCst), attempts + 1);
    }
    power.leave().await;
    let attempts = backend.acquired.load(Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(30)).await;
    tokio::task::yield_now().await;
    assert_eq!(backend.acquired.load(Ordering::SeqCst), attempts);
    assert_eq!(power.status().await.eligible_runs, 0);
    Ok(())
}
