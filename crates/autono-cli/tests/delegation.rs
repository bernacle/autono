//! Real daemon + CLI + production adapter, with shell-only provider executables.
//! No installed coding CLI, login or inference is invoked.
#![cfg(any(target_os = "macos", target_os = "linux"))]
use anyhow::{Context, Result, ensure};
use autono_protocol::{DaemonMetadata, engineering::*};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};
use uuid::Uuid;
fn cli(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_autono"));
    c.arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    c
}
async fn invoke(home: &Path, args: &[&str]) -> Result<String> {
    let o = tokio::time::timeout(Duration::from_secs(30), cli(home).args(args).output()).await??;
    ensure!(
        o.status.success(),
        "CLI {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Ok(String::from_utf8(o.stdout)?)
}
async fn ready(home: &Path, previous: Option<u32>) -> Result<DaemonMetadata> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(b) = std::fs::read(home.join("daemon.json"))
                && let Ok(m) = serde_json::from_slice::<DaemonMetadata>(&b)
                && Some(m.pid) != previous
            {
                return Ok(m);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}
fn start(home: &Path, root: &Path) -> Result<Child> {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("daemon.log"))?;
    Ok(cli(home)
        .arg("daemon")
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.join("bin").display()),
        )
        .env("AUTONO_FIXTURE_ROOT", root)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .process_group(0)
        .spawn()?)
}
async fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git")
        .arg("-c")
        .arg("user.name=Fixture")
        .arg("-c")
        .arg("user.email=fixture@localhost")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .args(args)
        .current_dir(repo)
        .output()
        .await?;
    ensure!(
        o.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Ok(String::from_utf8(o.stdout)?)
}
fn install_fixture(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir(root.join("bin"))?;
    let script = r##"#!/bin/sh
set -eu
if test "${1:-}" = --help; then
    printf '%s\n' '--print stream-json --json-schema --permission-prompts --safe-mode --bare --tools --strict-mcp-config --restricted --disallowedTools --disable-slash-commands --settings --setting-sources'
    exit 0
fi
# Fail if the production adapter accidentally restores tools or session resume.
want_tools=0
saw_tools=0
saw_bare=0
for arg do
    if test "$want_tools" = 1; then test -z "$arg"; want_tools=0; saw_tools=1; fi
    if test "$arg" = --tools; then want_tools=1; fi
    if test "$arg" = --bare; then saw_bare=1; fi
    test "$arg" != --safe-mode
    test "$arg" != --resume
    test "$arg" != --add-dir
done
test "$saw_tools" = 1
test "$saw_bare" = 1
label=${PWD##*/}
printf '%s\n' "$label" >> "$AUTONO_FIXTURE_ROOT/invocations"
if test -f "$AUTONO_FIXTURE_ROOT/slow"; then sleep 3; fi
case "$label" in
planner)
 result='{"human_requirements":[],"requirements":[{"id":"r","text":"Safe feature"}],"milestones":[{"id":"m","goal":"Feature","task_ids":["a","b","c"]}],"tasks":[{"id":"a","goal":"A","description":"Create A","acceptance_criteria":["A works"],"dependencies":[],"requirement_ids":["r"],"verification_expectations":["Configured check"],"role":"coder"},{"id":"b","goal":"B","description":"Create B","acceptance_criteria":["B works"],"dependencies":[],"requirement_ids":["r"],"verification_expectations":["Configured check"],"role":"coder"},{"id":"c","goal":"C","description":"Create C","acceptance_criteria":["C works"],"dependencies":["a"],"requirement_ids":["r"],"verification_expectations":["Configured check"],"role":"coder"}]}' ;;
task-*)
 id=${label#task-}
 case "$id" in
 a) content="/bin/echo A" ;;
 b) content="/bin/echo B" ;;
 c) content=". ./a.sh; /bin/echo C" ;;
 *) exit 94 ;;
 esac
 if test "$id" = b && ! test -f "$AUTONO_FIXTURE_ROOT/retried"; then content="/bin/echo incorrect"; touch "$AUTONO_FIXTURE_ROOT/retried"; fi
 result="{\"summary\":\"Implemented\",\"blocker\":\"\",\"edits\":[{\"path\":\"$id.sh\",\"content\":\"$content\\n\"}]}" ;;
review-*)
 id=${label#review-}; id=${id%-*}
 result="{\"accepted\":true,\"criteria\":[{\"criterion_id\":\"$id-C1\",\"criterion\":\"Works\",\"satisfied\":true,\"evidence\":\"Independent full diff and passing verification\"}],\"findings\":[],\"blocker\":\"\"}" ;;
*) exit 92 ;;
esac
uuid=$(if test -f /proc/sys/kernel/random/uuid; then cat /proc/sys/kernel/random/uuid; else /usr/bin/uuidgen; fi)
printf '{"type":"result","subtype":"success","session_id":"%s","structured_output":%s}\n' "$uuid" "$result"
"##;
    std::fs::write(root.join("bin/claude"), script)?;
    std::fs::set_permissions(
        root.join("bin/claude"),
        std::fs::Permissions::from_mode(0o755),
    )?;
    // A fake Codex name makes accidental provider selection an immediate failure.
    std::fs::write(root.join("bin/codex"), "#!/bin/sh\nexit 93\n")?;
    std::fs::set_permissions(
        root.join("bin/codex"),
        std::fs::Permissions::from_mode(0o755),
    )?;
    Ok(())
}
async fn run_view(http: &reqwest::Client, m: &DaemonMetadata, id: Uuid) -> Result<EngineeringRun> {
    Ok(http
        .get(format!("http://{}/v1/engineering/{id}", m.address))
        .bearer_auth(&m.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
async fn drained(http: &reqwest::Client, m: &DaemonMetadata, id: Uuid) -> Result<EngineeringRun> {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let r = run_view(http, m, id).await?;
            let health: serde_json::Value = http
                .get(format!("http://{}/v1/engineering/health", m.address))
                .bearer_auth(&m.token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if !r.status.active() && health["active_run"].is_null() {
                return Ok(r);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}
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
    daemon.kill().await?;
    if let Some(destination) = std::env::var_os("AUTONO_DELEGATION_EVIDENCE") {
        std::fs::copy(root.join("evidence.json"), PathBuf::from(destination))?;
        println!("retained fixture: {}", temp.keep().display());
    }
    Ok(())
}
