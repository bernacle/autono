#![cfg(unix)]

use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result};
use autono_protocol::{DaemonMetadata, Event, EventKind, Run, RunStatus};
use tokio::process::{Child, Command};

fn cli(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_autono"));
    command
        .arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

/// Seed the checkpoint after durable task admission but before provider launch.
/// A real daemon restart must reconcile it without installed agents or login.
#[tokio::test]
async fn daemon_restart_reconciles_engineering_checkpoints_without_agent_login() -> Result<()> {
    use autono_protocol::engineering::*;
    use sqlx::Connection;
    let dir = tempfile::tempdir()?;
    let home = tokio::fs::canonicalize(dir.path()).await?;
    let mut daemon = Daemon::start(&home)?;
    let old = ready(&home, None).await?;
    let id = uuid::Uuid::new_v4();
    let now = "2026-10-19T00:00:00Z".parse()?;
    let planner =
        autono_core::engineering::workspace::reference(&home, id, "planner", &"a".repeat(40));
    let workspace =
        autono_core::engineering::workspace::reference(&home, id, "task-t", &"a".repeat(40));
    tokio::fs::create_dir_all(&workspace.path).await?;
    tokio::fs::write(workspace.path.join("retained.txt"), "preserved after crash").await?;
    let task = PlannedTask {
        id: "t".into(),
        goal: "Feature".into(),
        description: "Implement feature".into(),
        acceptance_criteria: vec!["Feature works".into()],
        dependencies: vec![],
        requirement_ids: vec!["r".into()],
        verification_expectations: vec!["Tests pass".into()],
        role: AgentRole::Coder,
    };
    let run = EngineeringRun {
        id,
        revision: 0,
        status: EngineeringStatus::Running,
        created_at: now,
        updated_at: now,
        repository_git: None,
        repository: home.clone(),
        requested_base: "HEAD".into(),
        base_commit: "a".repeat(40),
        specification: "Feature".into(),
        settings: WorkflowSettings::default(),
        plan: Some(ExecutionPlan {
            human_requirements: vec![],
            requirements: vec![Requirement {
                id: "r".into(),
                text: "Feature".into(),
            }],
            milestones: vec![Milestone {
                id: "m".into(),
                goal: "Feature".into(),
                task_ids: vec!["t".into()],
            }],
            tasks: vec![task.clone()],
        }),
        approved_at: Some(now),
        planner: None,
        planning_workspace: planner,
        tasks: vec![EngineeringTask {
            plan: task,
            status: TaskStatus::Running,
            workspace: Some(workspace.clone()),
            attempts: vec![],
            artifact_commit: None,
        }],
        blockers: vec![],
        human_actions: vec![],
        preflight: None,
        integration_workspace: None,
        integration_verification: vec![],
        summary: "Agent launch checkpoint".into(),
    };
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(home.join("runtime.sqlite3")),
    )
    .await?;
    sqlx::query(
        "INSERT INTO engineering_runs(id,revision,status,document) VALUES(?,0,'RUNNING',?)",
    )
    .bind(id.to_string())
    .bind(serde_json::to_string(&run)?)
    .execute(&mut connection)
    .await?;
    daemon.kill().await?;
    daemon = Daemon::start(&home)?;
    ready(&home, Some(old.pid)).await?;
    let result: EngineeringRun =
        serde_json::from_str(&invoke(&home, &["summary", &id.to_string()]).await?)?;
    assert_eq!(result.status, EngineeringStatus::Interrupted);
    assert_eq!(result.tasks[0].status, TaskStatus::Interrupted);
    assert!(!result.blockers.is_empty());
    assert_eq!(
        tokio::fs::read_to_string(workspace.path.join("retained.txt")).await?,
        "preserved after crash"
    );
    let logs = invoke(&home, &["logs", "--engineering", &id.to_string()]).await?;
    assert!(logs.contains("interrupted"));
    daemon.kill().await?;
    Ok(())
}

struct Daemon {
    child: Child,
}
impl Daemon {
    fn start(home: &Path) -> Result<Self> {
        let child = cli(home)
            .arg("daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        Ok(Self { child })
    }
    async fn kill(&mut self) -> Result<()> {
        self.child.kill().await.context("kill daemon")
    }
}

async fn ready(home: &Path, previous_pid: Option<u32>) -> Result<DaemonMetadata> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(contents) = tokio::fs::read(home.join("daemon.json")).await
                && let Ok(metadata) = serde_json::from_slice::<DaemonMetadata>(&contents)
                && Some(metadata.pid) != previous_pid
            {
                return Ok(metadata);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("daemon did not publish readiness")?
}

async fn invoke(home: &Path, args: &[&str]) -> Result<String> {
    let output =
        tokio::time::timeout(Duration::from_secs(15), cli(home).args(args).output()).await??;
    anyhow::ensure!(
        output.status.success(),
        "CLI {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

async fn inspect(home: &Path, id: &str) -> Result<Run> {
    Ok(serde_json::from_str(&invoke(home, &["status", id]).await?)?)
}

async fn wait_output(metadata: &DaemonMetadata, id: &str) -> Result<()> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    let mut stream = client
        .get(format!(
            "http://{}/v1/runs/{id}/events/stream",
            metadata.address
        ))
        .bearer_auth(&metadata.token)
        .send()
        .await?
        .error_for_status()?;
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut buffer = String::new();
        while let Some(chunk) = stream.chunk().await? {
            buffer.push_str(std::str::from_utf8(&chunk)?);
            while let Some(end) = buffer.find("\n\n") {
                let frame: String = buffer.drain(..end + 2).collect();
                for line in frame.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        let event: Event = serde_json::from_str(data)?;
                        if matches!(event.kind, EventKind::Output { .. }) {
                            return Ok(());
                        }
                    }
                }
            }
        }
        anyhow::bail!("stream ended before command produced output")
    })
    .await?
}

/// The submitting CLI is a separate process that exits. A second CLI observes and
/// cancels its run. SIGKILL/restart must preserve logs and reconcile, never adopt a PID.
#[tokio::test]
async fn detached_submission_cancellation_and_crash_restart() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path();
    let mut daemon = Daemon::start(home)?;
    let metadata = ready(home, None).await?;
    let doctor = invoke(home, &["doctor"]).await?;
    assert!(doctor.contains("daemon: reachable"));
    assert!(doctor.contains("database: healthy"));
    let duplicate = cli(home).arg("daemon").output().await?;
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("another daemon"));
    let id = invoke(
        home,
        &[
            "run",
            "--no-keep-awake",
            "--",
            "sh",
            "-c",
            "echo still-supervised; exec sleep 300",
        ],
    )
    .await?
    .trim()
    .to_owned();
    wait_output(&metadata, &id).await?;
    assert_eq!(inspect(home, &id).await?.status, RunStatus::Running);
    assert_eq!(invoke(home, &["logs", &id]).await?, "still-supervised\n");
    invoke(home, &["cancel", &id]).await?;
    invoke(home, &["logs", "--follow", &id]).await?;
    assert_eq!(inspect(home, &id).await?.status, RunStatus::Cancelled);

    let crash = invoke(
        home,
        &[
            "run",
            "--no-keep-awake",
            "--",
            "sh",
            "-c",
            "echo before-crash; exec sleep 300",
        ],
    )
    .await?
    .trim()
    .to_owned();
    wait_output(&metadata, &crash).await?;
    let original = inspect(home, &crash).await?;
    // A harsh daemon death may leave the child alive. Retain its group only for test
    // cleanup; production reconciliation must never signal this persisted identifier.
    let cleanup = OrphanCleanup(original.pid.context("running process PID")?);
    use tokio::io::AsyncBufReadExt;
    let client = reqwest::Client::builder().no_proxy().build()?;
    let initial: Vec<Event> = client
        .get(format!(
            "http://{}/v1/runs/{crash}/events",
            metadata.address
        ))
        .bearer_auth(&metadata.token)
        .send()
        .await?
        .json()
        .await?;
    let cursor = initial.first().context("created event")?.sequence;
    let mut follower = cli(home)
        .args([
            "logs",
            "--events",
            "--follow",
            "--after",
            &cursor.to_string(),
            &crash,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut lines =
        tokio::io::BufReader::new(follower.stdout.take().context("follow stdout")?).lines();
    let mut followed = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await? {
            let event: Event = serde_json::from_str(&line)?;
            let output = matches!(event.kind, EventKind::Output { .. });
            followed.push(event);
            if output {
                return Ok::<_, anyhow::Error>(());
            }
        }
        anyhow::bail!("follower exited before replaying output")
    })
    .await??;
    daemon.kill().await?;
    daemon = Daemon::start(home)?;
    let restarted = ready(home, Some(metadata.pid)).await?;
    assert_ne!(metadata.token, restarted.token);
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(line) = lines.next_line().await? {
            followed.push(serde_json::from_str(&line)?);
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), follower.wait())
            .await??
            .success()
    );
    assert!(followed.iter().all(|event| event.sequence > cursor));
    assert!(
        followed
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert!(matches!(
        followed.last().map(|event| &event.kind),
        Some(EventKind::StateChanged {
            to: RunStatus::Interrupted,
            ..
        })
    ));
    let snapshot = invoke(
        home,
        &["logs", "--events", "--after", &cursor.to_string(), &crash],
    )
    .await?;
    assert!(snapshot.lines().all(|line| {
        serde_json::from_str::<Event>(line).is_ok_and(|event| event.sequence > cursor)
    }));
    let recovered = inspect(home, &crash).await?;
    let key = recovered
        .submission_key
        .context("durable submission key")?
        .to_string();
    assert_eq!(
        invoke(
            home,
            &[
                "run",
                "--submission-key",
                &key,
                "--no-keep-awake",
                "--",
                "sh",
                "-c",
                "echo before-crash; exec sleep 300"
            ]
        )
        .await?
        .trim(),
        crash
    );
    assert_eq!(recovered.status, RunStatus::Interrupted);
    // SAFETY: signal 0 does not deliver a signal. The surviving process confirms
    // restart did not silently terminate/adopt it through persisted metadata.
    assert_eq!(unsafe { libc::kill(i32::try_from(cleanup.0)?, 0) }, 0);
    assert!(
        recovered
            .detail
            .as_deref()
            .is_some_and(|s| s.contains("ownership"))
    );
    assert_eq!(invoke(home, &["logs", &crash]).await?, "before-crash\n");
    assert!(
        invoke(home, &["doctor"])
            .await?
            .contains("database: healthy")
    );
    drop(cleanup);
    daemon.kill().await?;
    Ok(())
}

struct OrphanCleanup(u32);
impl Drop for OrphanCleanup {
    fn drop(&mut self) {
        if let Ok(pid) = i32::try_from(self.0) {
            // SAFETY: this is the test's own freshly created child group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

#[tokio::test]
async fn shutdown_closes_incomplete_post_and_slow_sse_after_process_cleanup() -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir()?;
    tokio::fs::write(dir.path().join("config.toml"), "[runner]\nkeep_awake = false\n[daemon]\nbody_timeout_seconds = 30\ndrain_timeout_seconds = 1\n").await?;
    let mut daemon = Daemon::start(dir.path())?;
    let metadata = ready(dir.path(), None).await?;
    let id = invoke(
        dir.path(),
        &["run", "--", "sh", "-c", "echo ready; exec yes output"],
    )
    .await?
    .trim()
    .to_owned();
    wait_output(&metadata, &id).await?;
    let mut slow_sse = tokio::net::TcpStream::connect(metadata.address).await?;
    slow_sse.write_all(format!("GET /v1/runs/{id}/events/stream HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n\r\n", metadata.address, metadata.token).as_bytes()).await?;
    let mut incomplete = tokio::net::TcpStream::connect(metadata.address).await?;
    incomplete.write_all(format!("POST /v1/runs HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Length: 10000\r\n\r\n{{", metadata.address, metadata.token).as_bytes()).await?;
    assert_eq!(inspect(dir.path(), &id).await?.status, RunStatus::Running);
    // SAFETY: signal the daemon process created and retained by this test.
    anyhow::ensure!(
        unsafe { libc::kill(i32::try_from(metadata.pid)?, libc::SIGTERM) } == 0,
        "send SIGTERM"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(8), daemon.child.wait())
            .await
            .context("slow clients held daemon shutdown")??
            .success()
    );
    assert!(!tokio::fs::try_exists(dir.path().join("daemon.json")).await?);
    let store = autono_core::Store::inspect(&dir.path().join("runtime.sqlite3")).await?;
    assert_eq!(store.get(id.parse()?).await?.status, RunStatus::Interrupted);
    // Ownership was released and the same home can start another daemon.
    daemon = Daemon::start(dir.path())?;
    ready(dir.path(), Some(metadata.pid)).await?;
    daemon.kill().await?;
    Ok(())
}

#[tokio::test]
async fn persistent_finalization_failure_causes_nonzero_daemon_exit_and_cleans_metadata()
-> Result<()> {
    use autono_protocol::{HostStatus, RunPhase, RunView};
    use sqlx::Connection;
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir()?;
    tokio::fs::write(
        dir.path().join("config.toml"),
        "[runner]\nkeep_awake = false\n",
    )
    .await?;
    let gate = dir.path().join("exit-gate");
    let path = std::ffi::CString::new(gate.as_os_str().as_encoded_bytes())?;
    // SAFETY: valid path owned by this test.
    anyhow::ensure!(
        unsafe { libc::mkfifo(path.as_ptr(), 0o600) } == 0,
        "create FIFO"
    );
    let mut daemon = Daemon::start(dir.path())?;
    let metadata = ready(dir.path(), None).await?;
    let id = invoke(
        dir.path(),
        &[
            "run",
            "--cwd",
            dir.path().to_str().context("UTF-8 temporary directory")?,
            "--",
            "sh",
            "-c",
            "echo ready; read go < exit-gate; printf final-error >&2; exit 42",
        ],
    )
    .await?
    .trim()
    .to_owned();
    wait_output(&metadata, &id).await?;
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("runtime.sqlite3")),
    )
    .await?;
    sqlx::query("CREATE TRIGGER reject_terminal BEFORE UPDATE ON runs WHEN NEW.status IN ('FAILED','SUCCEEDED','CANCELLED','INTERRUPTED') BEGIN SELECT RAISE(ABORT, 'persistent finalization failure'); END").execute(&mut connection).await?;
    let mut gate = tokio::fs::OpenOptions::new().write(true).open(gate).await?;
    gate.write_all(b"go\n").await?;
    drop(gate);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    // Across a process boundary, bounded HTTP polling is the observable health
    // barrier. FIFO and SSE already synchronized command readiness and release.
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut poll = tokio::time::interval(Duration::from_millis(20));
        loop {
            poll.tick().await;
            let status: HostStatus = client
                .get(format!("http://{}/v1/status", metadata.address))
                .bearer_auth(&metadata.token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if !status.persistence.healthy {
                assert_eq!(status.admission.executing_runs, 0);
                assert_eq!(status.admission.finalizing_runs, 1);
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await??;
    let view: RunView = serde_json::from_str(&invoke(dir.path(), &["status", &id]).await?)?;
    let owner = view.runtime.context("CLI includes live finalization")?;
    assert_eq!(owner.phase, RunPhase::Finalizing);
    assert_eq!(
        owner.observed_result.context("observed exit")?.exit_code,
        Some(42)
    );
    assert!(
        invoke(dir.path(), &["status"])
            .await?
            .contains("FINALIZING")
    );
    assert!(
        invoke(dir.path(), &["doctor"])
            .await?
            .contains("persistence healthy=false")
    );
    // SAFETY: the test owns this daemon process.
    anyhow::ensure!(
        unsafe { libc::kill(i32::try_from(metadata.pid)?, libc::SIGTERM) } == 0,
        "signal daemon shutdown"
    );
    let exit = tokio::time::timeout(Duration::from_secs(15), daemon.child.wait())
        .await
        .context("failed finalization must not hold shutdown forever")??;
    assert!(
        !exit.success(),
        "graceful shutdown cannot succeed with unpersisted results"
    );
    assert!(!tokio::fs::try_exists(dir.path().join("daemon.json")).await?);
    let store = autono_core::Store::inspect(&dir.path().join("runtime.sqlite3")).await?;
    assert_eq!(store.get(id.parse()?).await?.status, RunStatus::Running);
    sqlx::query("DROP TRIGGER reject_terminal")
        .execute(&mut connection)
        .await?;
    // The failed exit explicitly warned that the in-memory result was not durable.
    // Restart may only report interruption, never invent the lost exit code.
    daemon = Daemon::start(dir.path())?;
    ready(dir.path(), Some(metadata.pid)).await?;
    let run = inspect(dir.path(), &id).await?;
    assert_eq!(run.status, RunStatus::Interrupted);
    assert_eq!(run.exit_code, None);
    daemon.kill().await?;
    Ok(())
}
