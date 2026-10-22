//! Real daemon acceptance with shell-only providers. No paid inference.
#![cfg(any(target_os = "macos", target_os = "linux"))]
#[allow(dead_code)]
#[path = "support/delegation.rs"]
mod support;
use anyhow::{Context, Result};
use autono_protocol::engineering::*;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use support::*;
use uuid::Uuid;
#[tokio::test]
async fn detached_cli_consent_real_daemon_safe_retry_integration_and_restart() -> Result<()> {
    if cfg!(target_os = "linux") && !Path::new("/usr/bin/bwrap").exists() {
        anyhow::bail!("install bubblewrap for autonomous acceptance");
    }
    let temp = tempfile::tempdir()?;
    let root = std::fs::canonicalize(temp.path())?;
    let home = root.join("home");
    let repo = root.join("repo");
    std::fs::create_dir(&home)?;
    std::fs::create_dir(&repo)?;
    install_fixture(&root)?;
    std::fs::write(
        home.join("config.toml"),
        r#"
[runner]
keep_awake = false
[agents.planner]
provider = "claude"
model = "fixture"
timeout_seconds = 30
[agents.coder]
provider = "claude"
model = "fixture"
timeout_seconds = 30
[agents.reviewer]
provider = "claude"
model = "fixture"
timeout_seconds = 30
[verification]
commands = [["/bin/sh", "-c", 'for f in a b c; do if test -f "$f.sh"; then case "$f" in a) expected=A;; b) expected=B;; c) expected="$(/bin/echo A; /bin/echo C)";; esac; actual="$(/bin/sh "./$f.sh")" || exit 1; test "$actual" = "$expected" || exit 1; fi; done']]
timeout_seconds = 10
[permissions.rules]
workspace_write = "REQUIRE_APPROVAL"
run_checks = "REQUIRE_APPROVAL"
"#,
    )?;
    git(&repo, &["init", "-b", "main"]).await?;
    std::fs::write(repo.join("README.md"), "Disposable safe seed\n")?;
    git(&repo, &["add", "."]).await?;
    git(&repo, &["commit", "-m", "seed"]).await?;
    let seed = git(&repo, &["rev-parse", "main"]).await?;
    let spec = root.join("feature.md");
    std::fs::write(
        &spec,
        "Implement shell scripts a.sh printing A and b.sh printing B independently; c.sh sources a.sh then prints C. Verify successful exits and exact output.\n",
    )?;
    let mut daemon = start(&home, &root)?;
    let m = ready(&home, None).await?;
    let http = reqwest::Client::builder().no_proxy().build()?;
    let id = Uuid::new_v4();
    let proposal = invoke(
        &home,
        &[
            "propose-delegation",
            spec.to_str().context("spec")?,
            "--repository",
            repo.to_str().context("repo")?,
            "--base",
            "main",
            "--plan-id",
            &id.to_string(),
            "--allow-file",
            "a.sh",
            "--allow-file",
            "b.sh",
            "--allow-file",
            "c.sh",
            "--max-runtime-seconds",
            "120",
            "--local-integration",
        ],
    )
    .await?;
    let c: DelegationContract = serde_json::from_str(&proposal)?;
    assert_eq!(c.request.submission.id, id);
    assert!(
        !root.join("invocations").exists(),
        "proposal must not invoke a provider"
    );
    let contract = root.join("consent.json");
    std::fs::write(&contract, proposal)?;
    let route = format!("http://{}/v1/delegations/{id}/authorize", m.address);
    assert_eq!(
        http.post(&route).json(&c).send().await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http.post(format!("http://{}/v1/delegations/propose", m.address))
            .json(&c.request)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http.post(&route)
            .bearer_auth(&m.token)
            .header("Origin", "http://untrusted.invalid")
            .json(&c)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let inspected: DelegationContract = http
        .get(format!("http://{}/v1/delegations/{id}", m.address))
        .bearer_auth(&m.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(inspected, c);
    let mut changed = c.clone();
    changed.settings.verification.commands = vec![vec!["/bin/echo".into(), "unapproved".into()]];
    assert_eq!(
        http.post(&route)
            .bearer_auth(&m.token)
            .json(&changed)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    invoke(
        &home,
        &[
            "authorize-delegation",
            contract.to_str().context("contract")?,
        ],
    )
    .await?;
    let duplicate: EngineeringRun = http
        .post(&route)
        .bearer_auth(&m.token)
        .json(&c)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(duplicate.id, id);
    // The submitting CLI has exited. No desktop/client is kept alive to admit transitions.
    let run = drained(&http, &m, id).await?;
    assert_eq!(
        run.status,
        EngineeringStatus::Completed,
        "{} {:?}",
        run.summary,
        run.human_actions
    );
    assert!(run.human_actions.is_empty());
    assert_eq!(run.tasks[1].attempts.len(), 2);
    assert_eq!(git(&repo, &["rev-parse", "main"]).await?, seed);
    assert!(!repo.join("a.sh").exists());
    let events: Vec<EngineeringEvent> = http
        .get(format!(
            "http://{}/v1/engineering/{id}/events?limit=500",
            m.address
        ))
        .bearer_auth(&m.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(!events.iter().any(|e| matches!(
        e.kind.as_str(),
        "approved" | "integration_requested" | "continued"
    )));
    let calls = std::fs::read_to_string(root.join("invocations"))?;
    assert_eq!(calls.lines().filter(|s| *s == "planner").count(), 1);
    assert_eq!(calls.lines().filter(|s| *s == "task-b").count(), 2);
    assert_eq!(
        calls.lines().filter(|s| s.starts_with("review-")).count(),
        3
    );
    // A real process restart preserves consent and evidence and never replays work.
    daemon.kill().await?;
    daemon = start(&home, &root)?;
    let restarted = ready(&home, Some(m.pid)).await?;
    let replay = invoke(
        &home,
        &[
            "authorize-delegation",
            contract.to_str().context("contract")?,
        ],
    )
    .await?;
    let replay: EngineeringRun = serde_json::from_str(&replay)?;
    assert_eq!(replay.delegation, run.delegation);
    assert_eq!(replay.status, EngineeringStatus::Completed);
    assert_eq!(std::fs::read_to_string(root.join("invocations"))?, calls);
    assert_eq!(
        run_view(&http, &restarted, id).await?.revision,
        run.revision
    );
    let evidence = json!({"run":run,"events":events,"seed_commit":seed.trim(),"provider_invocations":calls.lines().collect::<Vec<_>>(),"post_restart_replayed":false,"paid_inference":false});
    std::fs::write(
        root.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    println!(
        "unattended delegation evidence: {}",
        root.join("evidence.json").display()
    );
    // Crash during an actual provider invocation: restart preserves the grant
    // but must refuse to resume/replay its uncertain checkpoint.
    std::fs::write(root.join("slow"), "hold planner briefly")?;
    let mut crash_request = c.request.clone();
    crash_request.submission.id = Uuid::new_v4();
    let crash_contract: DelegationContract = http
        .post(format!(
            "http://{}/v1/delegations/propose",
            restarted.address
        ))
        .bearer_auth(&restarted.token)
        .json(&crash_request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let crash_id = crash_request.submission.id;
    http.post(format!(
        "http://{}/v1/delegations/{crash_id}/authorize",
        restarted.address
    ))
    .bearer_auth(&restarted.token)
    .json(&crash_contract)
    .send()
    .await?
    .error_for_status()?;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if std::fs::read_to_string(root.join("invocations"))
                .unwrap_or_default()
                .lines()
                .filter(|s| *s == "planner")
                .count()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    daemon.kill().await?;
    daemon = start(&home, &root)?;
    let after_crash = ready(&home, Some(restarted.pid)).await?;
    let interrupted = run_view(&http, &after_crash, crash_id).await?;
    assert_eq!(interrupted.status, EngineeringStatus::Interrupted);
    assert!(interrupted.delegation.is_some());
    let before = std::fs::read_to_string(root.join("invocations"))?;
    let replay: EngineeringRun = http
        .post(format!(
            "http://{}/v1/delegations/{crash_id}/authorize",
            after_crash.address
        ))
        .bearer_auth(&after_crash.token)
        .json(&crash_contract)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(replay.status, EngineeringStatus::Interrupted);
    assert_eq!(std::fs::read_to_string(root.join("invocations"))?, before);
    std::fs::remove_file(root.join("slow"))?;
    // Unavailable provider creates a durable human action instead of silently
    // switching to another installed executable or resetting authorization.
    std::fs::rename(root.join("bin/claude"), root.join("bin/claude-disabled"))?;
    let mut missing = crash_request;
    missing.submission.id = Uuid::new_v4();
    let missing_id = missing.submission.id;
    let missing_contract: DelegationContract = http
        .post(format!(
            "http://{}/v1/delegations/propose",
            after_crash.address
        ))
        .bearer_auth(&after_crash.token)
        .json(&missing)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    http.post(format!(
        "http://{}/v1/delegations/{missing_id}/authorize",
        after_crash.address
    ))
    .bearer_auth(&after_crash.token)
    .json(&missing_contract)
    .send()
    .await?
    .error_for_status()?;
    let unavailable = drained(&http, &after_crash, missing_id).await?;
    assert_eq!(
        unavailable.status,
        EngineeringStatus::Blocked,
        "{}",
        unavailable.summary
    );
    assert!(!unavailable.human_actions.is_empty());
    assert_eq!(std::fs::read_to_string(root.join("invocations"))?, before);
    std::fs::write(
        root.join("adversarial-evidence.json"),
        serde_json::to_vec_pretty(&json!({"interrupted":interrupted,"unavailable":unavailable}))?,
    )?;
    // P2: exercise the production adapter's help-only capability probes in an
    // inactive preflight. The first help process is gated across the deadline.
    std::fs::rename(root.join("bin/claude-disabled"), root.join("bin/claude"))?;
    std::fs::write(root.join("hold-preflight"), "pause before execution")?;
    let mut preparation = c.request.clone();
    preparation.submission.id = Uuid::new_v4();
    preparation.max_runtime_seconds = 6;
    let preparation_id = preparation.submission.id;
    let contract: DelegationContract = http
        .post(format!(
            "http://{}/v1/delegations/propose",
            after_crash.address
        ))
        .bearer_auth(&after_crash.token)
        .json(&preparation)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    http.post(format!(
        "http://{}/v1/delegations/{preparation_id}/authorize",
        after_crash.address
    ))
    .bearer_auth(&after_crash.token)
    .json(&contract)
    .send()
    .await?
    .error_for_status()?;
    let paused = drained(&http, &after_crash, preparation_id).await?;
    assert_eq!(
        paused.status,
        EngineeringStatus::AwaitingApproval,
        "{}",
        paused.summary
    );
    let expiry = paused
        .delegation
        .as_ref()
        .context("authorization")?
        .expires_at;
    tokio::time::sleep(
        (expiry - chrono::Utc::now() - chrono::Duration::milliseconds(900))
            .to_std()
            .context("preparation fixture must pause before expiry")?,
    )
    .await;
    let probes = std::fs::read_to_string(root.join("probe-invocations"))?;
    std::fs::write(root.join("delay-probe"), "gate capability response")?;
    let request = http
        .post(format!(
            "http://{}/v1/engineering/{preparation_id}/preflight",
            after_crash.address
        ))
        .bearer_auth(&after_crash.token);
    let pending = tokio::spawn(async move { request.send().await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.join("probe-started").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    tokio::time::sleep(
        (expiry - chrono::Utc::now()).to_std().unwrap_or_default() + Duration::from_millis(50),
    )
    .await;
    std::fs::write(root.join("release-probe"), "release after expiry")?;
    let response = tokio::time::timeout(Duration::from_secs(5), pending).await???;
    assert!(!response.status().is_success());
    // Existing API errors conceal internal diagnostics; the authenticated run
    // view below must carry the precise, durable authorization reason.
    let _ = response.text().await?;
    let expired = drained(&http, &after_crash, preparation_id).await?;
    assert!(
        expired.summary.contains("runtime budget"),
        "{}",
        expired.summary
    );
    assert_eq!(expired.preflight, paused.preflight);
    assert_eq!(
        std::fs::read_to_string(root.join("probe-invocations"))?
            .lines()
            .count(),
        probes.lines().count() + 1,
        "no second capability process after expiry"
    );
    assert!(expired.tasks.iter().all(|t| t.attempts.is_empty()));
    assert_eq!(git(&repo, &["rev-parse", "main"]).await?, seed);
    std::fs::write(
        root.join("lifecycle-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"expired_preflight":expired,"probe_launches_during_preparation":1,"post_expiry_probe_launches":0,"original_main_unchanged":true,"paid_inference":false}),
        )?,
    )?;
    daemon.kill().await?;
    if let Some(destination) = std::env::var_os("AUTONO_DELEGATION_EVIDENCE") {
        std::fs::copy(root.join("evidence.json"), PathBuf::from(destination))?;
        println!("retained fixture: {}", temp.keep().display());
    }
    Ok(())
}
