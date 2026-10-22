//! Real daemon + CLI + production adapter, with shell-only provider executables.
//! No installed coding CLI, login or inference is invoked.
#![cfg(any(target_os = "macos", target_os = "linux"))]
use anyhow::{Result, ensure};
use autono_protocol::{DaemonMetadata, engineering::*};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::{Child, Command};
use uuid::Uuid;
pub fn cli(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_autono"));
    c.arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    c
}
pub async fn invoke(home: &Path, args: &[&str]) -> Result<String> {
    let o = tokio::time::timeout(Duration::from_secs(30), cli(home).args(args).output()).await??;
    ensure!(
        o.status.success(),
        "CLI {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Ok(String::from_utf8(o.stdout)?)
}
pub async fn ready(home: &Path, previous: Option<u32>) -> Result<DaemonMetadata> {
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
pub fn start(home: &Path, root: &Path) -> Result<Child> {
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
pub async fn git(repo: &Path, args: &[&str]) -> Result<String> {
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
pub fn install_fixture(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir(root.join("bin"))?;
    let script = r##"#!/bin/sh
set -eu
if test "${1:-}" = --help; then
    printf '%s\n' probe >> "$AUTONO_FIXTURE_ROOT/probe-invocations"
    if test -f "$AUTONO_FIXTURE_ROOT/delay-probe"; then
        touch "$AUTONO_FIXTURE_ROOT/probe-started"
        while ! test -f "$AUTONO_FIXTURE_ROOT/release-probe"; do sleep 0.01; done
    fi
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
if test "$label" = planner && test -f "$AUTONO_FIXTURE_ROOT/hold-preflight"; then
    result=$(printf '%s' "$result" | sed 's/"human_requirements":\[\]/"human_requirements":[{"kind":"product_decision","reason":"Fixture decision","requested_action":"Choose behavior","affected_tasks":[],"blocking":true,"evidence":"Deterministic hold","confidence":100,"permission":null}]/')
fi
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
pub async fn run_view(
    http: &reqwest::Client,
    m: &DaemonMetadata,
    id: Uuid,
) -> Result<EngineeringRun> {
    Ok(http
        .get(format!("http://{}/v1/engineering/{id}", m.address))
        .bearer_auth(&m.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
pub async fn drained(
    http: &reqwest::Client,
    m: &DaemonMetadata,
    id: Uuid,
) -> Result<EngineeringRun> {
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
