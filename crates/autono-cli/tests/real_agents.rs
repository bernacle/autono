//! Explicit opt-in only. Uses installed CLI login, spends inference, retains all artifacts.
#![cfg(unix)]
use anyhow::{Context, Result, ensure};
use autono_protocol::{
    DaemonMetadata,
    engineering::{EngineeringRun, EngineeringStatus},
};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use uuid::Uuid;

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        // This Child is live ownership, never a persisted PID.
        // SAFETY: this live Child owns its PID until reaped.
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        for _ in 0..100 {
            if self.0.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn git(repo: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "user.name=Autono acceptance",
            "-c",
            "user.email=acceptance@localhost",
        ])
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::null())
        .output()?;
    ensure!(
        output.status.success(),
        "Git setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
async fn wait(
    http: &reqwest::Client,
    metadata: &DaemonMetadata,
    id: Uuid,
) -> Result<EngineeringRun> {
    tokio::time::timeout(Duration::from_secs(900), async {
        loop {
            let run: EngineeringRun = http
                .get(format!("http://{}/v1/engineering/{id}", metadata.address))
                .bearer_auth(&metadata.token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if !run.status.active() {
                let health: serde_json::Value = http
                    .get(format!("http://{}/v1/engineering/health", metadata.address))
                    .bearer_auth(&metadata.token)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if health["active_run"].is_null() {
                    return Ok::<_, anyhow::Error>(run);
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .context("real-agent acceptance deadline expired; inspect retained daemon logs")?
}
#[tokio::test]
#[ignore = "explicitly opt in with AUTONO_REAL_AGENT_ACCEPTANCE=1; uses real Codex/Claude login and inference"]
async fn real_codex_coder_and_independent_claude_reviewer() -> Result<()> {
    ensure!(
        std::env::var("AUTONO_REAL_AGENT_ACCEPTANCE").as_deref() == Ok("1"),
        "set AUTONO_REAL_AGENT_ACCEPTANCE=1 to authorize real inference"
    );
    let root = std::env::temp_dir().join(format!("autono-real-agent-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root)?;
    let root = std::fs::canonicalize(root)?;
    let repo = root.join("sample");
    std::fs::create_dir(&repo)?;
    let seed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config-validator");
    for name in [
        "README.md",
        "cli.py",
        "test_cli.py",
        "feature.md",
        ".gitignore",
    ] {
        std::fs::copy(seed.join(name), repo.join(name))?;
    }
    git(&repo, &["init", "-b", "main"])?;
    git(&repo, &["add", "."])?;
    git(&repo, &["commit", "-m", "working acceptance seed"])?;
    let baseline = Command::new("python3")
        .args(["-m", "unittest", "discover", "-v"])
        .current_dir(&repo)
        .output()?;
    ensure!(baseline.status.success(), "seed tests failed");
    let home = root.join("home");
    std::fs::create_dir(&home)?;
    std::fs::write(
        home.join("config.toml"),
        "[runner]\nkeep_awake=false\n[agents.planner]\nprovider=\"claude\"\ntimeout_seconds=90\n[agents.coder]\nprovider=\"codex\"\ntimeout_seconds=180\n[agents.reviewer]\nprovider=\"claude\"\ntimeout_seconds=120\n[execution]\nmax_parallel=1\nmax_attempts=2\nmax_review_cycles=2\n[verification]\ncommands=[[\"python3\",\"-m\",\"unittest\",\"discover\",\"-v\"]]\ntimeout_seconds=30\n",
    )?;
    let log = std::fs::File::create(root.join("daemon.log"))?;
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_autono"))
            .arg("--home")
            .arg(&home)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?,
    );
    eprintln!(
        "Retained real-agent acceptance directory: {}",
        root.display()
    );
    let metadata: DaemonMetadata = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(bytes) = tokio::fs::read(home.join("daemon.json")).await
                && let Ok(m) = serde_json::from_slice(&bytes)
            {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()?;
    // A separate submitting CLI exits before planning finishes.
    let submission = Command::new(env!("CARGO_BIN_EXE_autono"))
        .arg("--home")
        .arg(&home)
        .arg("plan")
        .arg(repo.join("feature.md"))
        .arg("--repository")
        .arg(&repo)
        .output()?;
    ensure!(
        submission.status.success(),
        "plan submission failed: {}",
        String::from_utf8_lossy(&submission.stderr)
    );
    let id: Uuid = std::str::from_utf8(&submission.stdout)?.trim().parse()?;
    let run = wait(&http, &metadata, id).await?;
    std::fs::write(
        root.join("planning-result.json"),
        serde_json::to_vec_pretty(&run)?,
    )?;
    ensure!(
        run.status == EngineeringStatus::AwaitingApproval,
        "real planner did not produce an approved-ready plan: {:?}: {}; retained at {}",
        run.status,
        run.summary,
        root.display()
    );
    let submit = |action: &str| {
        Command::new(env!("CARGO_BIN_EXE_autono"))
            .arg("--home")
            .arg(&home)
            .arg(action)
            .arg(id.to_string())
            .output()
    };
    let result = submit("execute")?;
    ensure!(
        result.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run = wait(&http, &metadata, id).await?;
    std::fs::write(
        root.join("execution-result.json"),
        serde_json::to_vec_pretty(&run)?,
    )?;
    ensure!(
        run.status == EngineeringStatus::Verified,
        "real execution did not verify: {:?}: {}; retained at {}",
        run.status,
        run.summary,
        root.display()
    );
    let result = submit("integrate")?;
    ensure!(
        result.status.success(),
        "integrate failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run = wait(&http, &metadata, id).await?;
    std::fs::write(
        root.join("final-result.json"),
        serde_json::to_vec_pretty(&run)?,
    )?;
    ensure!(
        run.status == EngineeringStatus::Completed,
        "real integration failed: {:?}: {}",
        run.status,
        run.summary
    );
    eprintln!("{}", run.summary);
    Ok(())
}
