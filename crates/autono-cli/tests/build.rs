//! End-to-end A1: automatic source discovery, v2 consent, real authenticated daemon, SQLite, Git, sandbox and CLI.
#![cfg(any(target_os = "macos", target_os = "linux"))]
#[allow(dead_code)]
#[path = "support/delegation.rs"]
mod support;
use anyhow::{Context, Result, ensure};
use autono_protocol::engineering::*;
use serde_json::json;
use std::{path::Path, time::Duration};
use support::*;
use uuid::Uuid;

fn install(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    install_fixture(root)?;
    let script = r##"#!/bin/sh
set -eu
if test "${1:-}" = --help; then
 printf '%s\n' '--print stream-json --json-schema --permission-prompts --safe-mode --bare --tools --strict-mcp-config --restricted --disallowedTools --disable-slash-commands --settings --setting-sources'
 exit 0
fi
want=0; saw=0
for arg do
 if test "$want" = 1; then test -z "$arg"; saw=1; want=0; fi
 if test "$arg" = --tools; then want=1; fi
 test "$arg" != --resume; test "$arg" != --add-dir; test "$arg" != --safe-mode
 prompt=$arg
done
test "$saw" = 1
case "$prompt" in *'Default export format is CSV.'*'Current run decision ledger'*) : ;; *'Current run decision ledger'*'Default export format is CSV.'*) : ;; *) exit 98;; esac
case "$prompt" in *SENSITIVE_SENTINEL_DO_NOT_READ*|*PROTECTED_INSTRUCTIONS_DO_NOT_READ*) exit 88;; esac
label=${PWD##*/}
printf '%s\n' "$label" >> "$AUTONO_FIXTURE_ROOT/invocations"
case "$label" in
 planner) result=$(cat "$AUTONO_FIXTURE_ROOT/plan.json");;
 task-*)
  if test -f "$AUTONO_FIXTURE_ROOT/hold"; then touch "$AUTONO_FIXTURE_ROOT/coder-started"; while test -f "$AUTONO_FIXTURE_ROOT/hold"; do sleep 0.01; done; fi
  if test "$label" = task-tests && ! test -f "$AUTONO_FIXTURE_ROOT/test-retried"; then
    touch "$AUTONO_FIXTURE_ROOT/test-retried"
    result='{"summary":"First test candidate","blocker":"","decisions":[],"edits":[{"path":"tests/test_export.sh","content":"exit 1\n"}]}'
  else result=$(cat "$AUTONO_FIXTURE_ROOT/$label.json"); fi;;
 review-*)
  case "$prompt" in *'Original delegated feature specification:'*'Add CSV export to the reporting module, including validation and tests.'*) : ;; *) exit 97;; esac
  id=${label#review-}; id=${id%-*}
  case "$prompt" in *'"chosen":"name,value"'*'"content_sha256"'*) : ;; *) exit 95;; esac
  case "$prompt" in *'"diff_excerpt"'*) : ;; *) exit 94;; esac
  case "$id" in
    format) case "$prompt" in *'format_report'*'name,value'*) : ;; *) exit 93;; esac ;;
    validate) case "$prompt" in *'validate_report'*'return 1'*) : ;; *) exit 92;; esac ;;
    export) case "$prompt" in *'export_report'*'validate_report'*) : ;; *) exit 91;; esac ;;
    tests) case "$prompt" in *'tests/test_export.sh'*'comma,name'*) : ;; *) exit 90;; esac ;;
    docs) case "$prompt" in *'src/reporting/export.txt'*'default to CSV'*) : ;; *) exit 89;; esac ;;
  esac
  decision_reviews='[]'
  decision_id=$(printf '%s\n' "$prompt" | sed -n 's/^Consumed decision ID: //p')
  if test -n "$decision_id"; then
    decision_reviews="[{\"decision_id\":\"$decision_id\",\"implemented\":true,\"evidence\":\"The candidate implements the CSV revision in the affected task; full diff independently inspected.\"}]"
  fi
  result="{\"decision_reviews\":$decision_reviews,\"accepted\":true,\"criteria\":[{\"criterion_id\":\"$id-C1\",\"criterion\":\"Works\",\"satisfied\":true,\"evidence\":\"Full candidate diff, original requirements, CSV decision and configured checks reviewed independently\"}],\"findings\":[],\"blocker\":\"\"}";;
 *) exit 96;;
esac
if test "$label" = review-docs-1 && ! test -f "$AUTONO_FIXTURE_ROOT/review-retried"; then
 touch "$AUTONO_FIXTURE_ROOT/review-retried"
 result=$(printf '%s' "$result" | sed 's/"accepted":true/"accepted":false/')
fi
uuid=$(if test -f /proc/sys/kernel/random/uuid; then cat /proc/sys/kernel/random/uuid; else /usr/bin/uuidgen; fi)
printf '{"type":"result","subtype":"success","session_id":"%s","structured_output":%s}\n' "$uuid" "$result"
"##;
    std::fs::write(root.join("bin/claude"), script)?;
    std::fs::set_permissions(
        root.join("bin/claude"),
        std::fs::Permissions::from_mode(0o755),
    )?;
    let ids = ["format", "validate", "export", "tests", "docs"];
    let tasks: Vec<_> = ids.iter().map(|id| json!({"id":id,"goal":format!("Report {id}"),"description":format!("Implement {id} using the recorded default format"),"acceptance_criteria":["Report export behavior satisfies the original requirements and configured checks"],"dependencies": match *id { "export" => vec!["format","validate"], "tests" | "docs" => vec!["export"], _ => vec![] },"requirement_ids":["report"],"verification_expectations":["Configured sandboxed checks"],"role":"coder"})).collect();
    let proposal = json!({"key":"export-format","question":"Which CSV column order?","chosen":"name,value","alternatives":["value,name"],"rationale":"Preserve the existing report column order in CSV exports.","kind":"default_value","evidence":[{"file":"src/format.sh","excerpt":"Report columns are name,value.","supports":"name,value"}],"requirements":["Add CSV export to the reporting module, including validation and tests."],"assumptions":["The existing column order applies to CSV exports."],"uncertainty":[],"affected_files":["src/format.sh","src/reporting/export.sh","tests/test_export.sh","src/reporting/export.txt"],"affected_tasks":["format","export","tests","docs"],"supersedes":null});
    std::fs::write(
        root.join("plan.json"),
        serde_json::to_vec(
            &json!({"decisions":[proposal],"human_requirements":[],"requirements":[{"id":"report","text":"Add CSV export to the reporting module, including validation and tests."}],"milestones":[{"id":"report","goal":"Validated export","task_ids":ids}],"tasks":tasks}),
        )?,
    )?;
    let edits = [
        (
            "format",
            "src/format.sh",
            "# Default export format is CSV.\nformat_report() { printf 'name,value\\n%s,%s\\n' \"$1\" \"$2\"; }\n",
        ),
        (
            "validate",
            "src/validate.sh",
            "validate_report() {\n test \"$#\" -eq 2 && test -n \"$1\" || return 1\n case \"$1\" in *,*) return 1;; esac\n case \"$2\" in ''|*[!0-9]*) return 1;; esac\n}\n",
        ),
        (
            "export",
            "src/reporting/export.sh",
            ". ./src/format.sh\n. ./src/validate.sh\nexport_report() { validate_report \"$@\" || return 1; format_report \"$@\"; }\n",
        ),
        (
            "tests",
            "tests/test_export.sh",
            "set -eu\n. ./src/reporting/export.sh\nexpected=$(printf 'name,value\\nalpha,42')\ntest \"$(export_report alpha 42)\" = \"$expected\"\nif export_report '' 42; then exit 1; fi\nif export_report alpha invalid; then exit 1; fi\nif export_report 'comma,name' 42; then exit 1; fi\n",
        ),
        (
            "docs",
            "src/reporting/export.txt",
            "Report exports default to CSV with name,value header. A name and nonnegative integer value are required; comma-containing names are rejected.\n",
        ),
    ];
    for (id, file, content) in edits {
        std::fs::write(
            root.join(format!("task-{id}.json")),
            serde_json::to_vec(
                &json!({"summary":"Implemented report export task","blocker":"","decisions":[],"edits":[{"path":file,"content":content}]}),
            )?,
        )?;
    }
    Ok(())
}

// Authorization is keyed by the immutable contract. A slow/lost HTTP receipt
// may be replayed without starting another workflow or renewing its deadline.
async fn authorize(home: &Path, consent: &Path) -> Result<String> {
    for attempt in 0..3 {
        let result = invoke(
            home,
            &["authorize-delegation", consent.to_str().context("consent")?],
        )
        .await;
        match result {
            Err(error) if attempt < 2 && format!("{error:#}").contains("operation timed out") => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            result => return result,
        }
    }
    unreachable!("bounded authorization receipt replay")
}

#[tokio::test]
async fn build_discovers_csv_sources_creates_files_reviews_integrates_and_never_replays()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = std::fs::canonicalize(temp.path())?;
    let _cleanup = if std::env::var_os("AUTONO_BUILD_KEEP").is_some() {
        println!("retained build fixture: {}", temp.keep().display());
        None
    } else {
        Some(temp)
    };
    let home = root.join("home");
    let repo = root.join("repo");
    std::fs::create_dir(&home)?;
    std::fs::create_dir(&repo)?;
    install(&root)?;
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
commands = [["/bin/sh", "-c", '/bin/sh tests/test_reporting.sh || exit 1; for f in src/format.sh src/validate.sh src/reporting/export.sh tests/test_export.sh; do if test -f "$f"; then /bin/sh -n "$f" || exit 1; fi; done; if test -f tests/test_export.sh; then /bin/sh ./tests/test_export.sh || exit 1; fi; if test "${PWD##*/}" = integration; then test -f src/reporting/export.txt && test -f tests/test_export.sh || exit 1; fi']]
timeout_seconds = 10
[permissions.rules]
workspace_write = "REQUIRE_APPROVAL"
run_checks = "REQUIRE_APPROVAL"
"#,
    )?;
    git(&repo, &["init", "-b", "main"]).await?;
    std::fs::create_dir_all(repo.join("src/reporting"))?;
    std::fs::create_dir_all(repo.join("tests"))?;
    std::fs::write(
        repo.join("src/format.sh"),
        "# Default export format is CSV.\n# Report columns are name,value.\n",
    )?;
    std::fs::write(repo.join("src/.env"), "SENSITIVE_SENTINEL_DO_NOT_READ")?;
    std::fs::write(
        repo.join("src/AGENTS.md"),
        "PROTECTED_INSTRUCTIONS_DO_NOT_READ",
    )?;
    git(&repo, &["add", "."]).await?;
    git(&repo, &["commit", "-m", "CSV convention"]).await?;
    std::fs::write(
        repo.join("src/reporting/report.sh"),
        "report_title() { printf 'Reporting\\n'; }\n",
    )?;
    std::fs::write(
        repo.join("tests/test_reporting.sh"),
        "set -eu\n. ./src/reporting/report.sh\ntest \"$(report_title)\" = Reporting\n",
    )?;
    git(&repo, &["add", "."]).await?;
    git(
        &repo,
        &["commit", "-m", "Existing reporting module and tests"],
    )
    .await?;
    let seed = git(&repo, &["rev-parse", "main"]).await?;
    let spec = root.join("feature.md");
    std::fs::write(
        &spec,
        "Add CSV export to the reporting module, including validation and tests.",
    )?;
    let mut daemon = start(&home, &root)?;
    let m = ready(&home, None).await?;
    let http = reqwest::Client::builder().no_proxy().build()?;
    let id = Uuid::new_v4();
    let mut args = vec![
        "build",
        "feature.md",
        "--repository",
        "repo",
        "--base",
        "main",
        "--plan-id",
    ];
    let id_text = id.to_string();
    args.push(&id_text);
    args.extend(["--max-runtime-seconds", "120", "--proposal-only"]);
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        cli(&home).current_dir(&root).args(&args).output(),
    )
    .await??;
    ensure!(
        output.status.success(),
        "relative-path Build proposal failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let proposal = String::from_utf8(output.stdout)?;
    let contract: DelegationContract = serde_json::from_str(&proposal)?;
    assert_eq!(contract.version, 2);
    assert!(
        contract.request.writable_files.is_empty(),
        "no generated exact-file allowlist"
    );
    assert!(contract.source_inventory.contains(&"src/format.sh".into()));
    assert!(
        contract
            .request
            .source_scope
            .as_ref()
            .is_some_and(|s| s.allow_create)
    );
    assert!(
        !root.join("invocations").exists(),
        "discovery must be inference-free"
    );
    let consent = root.join("consent.json");
    std::fs::write(&consent, proposal)?;
    authorize(&home, &consent).await?;
    let run = drained(&http, &m, id).await?;
    assert_eq!(
        run.status,
        EngineeringStatus::Completed,
        "{} {:?}",
        run.summary,
        run.human_actions
    );
    assert!(run.human_actions.is_empty());
    assert!(
        run.tasks
            .iter()
            .find(|t| t.plan.id == "tests")
            .context("tests")?
            .attempts
            .len()
            >= 2
    );
    assert!(
        run.handoff
            .as_ref()
            .is_some_and(|h| h.completion_requirements_satisfied)
    );
    assert_eq!(run.tasks.len(), 5);
    assert_eq!(run.decisions.len(), 1);
    assert!(run.plan.as_ref().context("plan")?.decisions.is_empty());
    assert!(
        serde_json::to_value(&run.plan)?["decisions"].is_null(),
        "frozen plans preserve the legacy wire shape"
    );
    let decision = &run.decisions[0];
    assert_eq!(decision.proposal.chosen, "name,value");
    assert!(decision.verified());
    assert_eq!(decision.evidence.len(), 1);
    assert_eq!(
        decision.evidence[0].excerpt,
        "Report columns are name,value."
    );
    assert!(run.tasks.iter().all(|t| {
        t.attempts.last().is_some_and(|a| {
            a.verified_tree == a.preserved_tree
                && a.review.as_ref().is_some_and(ReviewResult::accepts)
        })
    }));
    let ledger_url = format!("http://{}/v1/engineering/{id}/decisions", m.address);
    assert_eq!(
        http.get(&ledger_url).send().await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let ledger: Vec<DecisionRecord> = http
        .get(&ledger_url)
        .bearer_auth(&m.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(ledger, run.decisions);
    let cli_json = invoke(&home, &["decisions", &id_text, "--json"]).await?;
    assert_eq!(
        serde_json::from_str::<Vec<DecisionRecord>>(&cli_json)?,
        ledger
    );
    assert!(
        invoke(&home, &["decisions", &id_text])
            .await?
            .contains("independently verified")
    );
    let events: Vec<EngineeringEvent> = http
        .get(format!(
            "http://{}/v1/engineering/{id}/events?limit=1000",
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
        "approved" | "continued" | "integration_requested"
    )));
    assert_eq!(git(&repo, &["rev-parse", "main"]).await?, seed);
    let calls = std::fs::read_to_string(root.join("invocations"))?;
    assert_eq!(calls.lines().filter(|s| *s == "planner").count(), 1);
    daemon.kill().await?;
    daemon = start(&home, &root)?;
    let restarted = ready(&home, Some(m.pid)).await?;
    authorize(&home, &consent).await?;
    let restored = run_view(&http, &restarted, id).await?;
    assert_eq!(restored.decisions, ledger);
    assert_eq!(
        restored.delegation, run.delegation,
        "receipt replay must not renew authority or its deadline"
    );
    assert_eq!(std::fs::read_to_string(root.join("invocations"))?, calls);
    // Interrupt after the decision commit but before coder output/edits.
    std::fs::write(root.join("hold"), "hold coder")?;
    let mut request = contract.request.clone();
    request.submission.id = Uuid::new_v4();
    let interrupted_id = request.submission.id;
    let c: DelegationContract = http
        .post(format!(
            "http://{}/v1/delegations/propose",
            restarted.address
        ))
        .bearer_auth(&restarted.token)
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let route = format!(
        "http://{}/v1/delegations/{interrupted_id}/authorize",
        restarted.address
    );
    http.post(&route)
        .bearer_auth(&restarted.token)
        .json(&c)
        .send()
        .await?
        .error_for_status()?;
    tokio::time::timeout(Duration::from_secs(30), async {
        while !root.join("coder-started").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    let saved = run_view(&http, &restarted, interrupted_id).await?;
    assert_eq!(saved.decisions.len(), 1);
    assert!(!saved.decisions[0].verified());
    daemon.kill().await?;
    // Let the fake orphan return; the dead daemon cannot apply its output.
    std::fs::remove_file(root.join("hold"))?;
    daemon = start(&home, &root)?;
    let final_host = ready(&home, Some(restarted.pid)).await?;
    let interrupted = run_view(&http, &final_host, interrupted_id).await?;
    assert_eq!(interrupted.status, EngineeringStatus::Interrupted);
    assert_eq!(interrupted.decisions, saved.decisions);
    let before = std::fs::read_to_string(root.join("invocations"))?;
    let replay: EngineeringRun = http
        .post(format!(
            "http://{}/v1/delegations/{interrupted_id}/authorize",
            final_host.address
        ))
        .bearer_auth(&final_host.token)
        .json(&c)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(replay.decisions, interrupted.decisions);
    assert_eq!(std::fs::read_to_string(root.join("invocations"))?, before);
    ensure!(
        run.delegation.as_ref().context("auth")?.contract == contract,
        "decisions changed delegation"
    );
    let evidence = json!({"run":run,"events":events,"seed_commit":seed.trim(),"provider_invocations":calls.lines().collect::<Vec<_>>(),"restart_replayed":false,"interrupted_run":interrupted,"paid_inference":false});
    if let Some(destination) = std::env::var_os("AUTONO_BUILD_EVIDENCE") {
        std::fs::write(destination, serde_json::to_vec_pretty(&evidence)?)?;
    }
    daemon.kill().await?;
    Ok(())
}

#[tokio::test]
async fn v2_denials_and_deferrable_choices_preserve_independent_verified_work() -> Result<()> {
    let mut evidence = vec![];
    for scenario in [
        "outside",
        "credential",
        "configuration",
        "traversal",
        "command",
        "ignored",
        "category_b",
        "deferrable",
    ] {
        let temp = tempfile::tempdir()?;
        let root = std::fs::canonicalize(temp.path())?;
        let home = root.join("home");
        let repo = root.join("repo");
        std::fs::create_dir(&home)?;
        std::fs::create_dir_all(repo.join("src"))?;
        install(&root)?;
        std::fs::write(
            home.join("config.toml"),
            r#"
[runner]
keep_awake = false
[agents.planner]
provider = "claude"
model = "fixture"
[agents.coder]
provider = "claude"
model = "fixture"
[agents.reviewer]
provider = "claude"
model = "fixture"
[verification]
commands = [["/bin/sh", "-c", 'for f in src/format.sh src/validate.sh; do if test -f "$f"; then /bin/sh -n "$f" || exit 1; fi; done']]
timeout_seconds = 10
"#,
        )?;
        git(&repo, &["init", "-b", "main"]).await?;
        std::fs::write(
            repo.join("src/format.sh"),
            "# Default export format is CSV.\n# Report columns are name,value.\n",
        )?;
        if scenario == "ignored" {
            std::fs::write(repo.join(".gitignore"), "src/validate.sh\n")?;
        }
        git(&repo, &["add", "."]).await?;
        git(&repo, &["commit", "-m", "seed"]).await?;
        let mut coder = json!({"summary":"Proposed source change","blocker":"","decisions":[],"edits":[{"path":"src/validate.sh","content":"# must not be applied\n"}]});
        match scenario {
            "ignored" => {}
            "category_b" => {
                coder["decisions"] = json!([{"key":"storage-choice","question":"Replace reporting storage?","chosen":"SQLite","alternatives":["remote storage"],"rationale":"A significant storage choice requires human input.","kind":"significant","evidence":[],"requirements":["Add CSV export to the reporting module, including validation and tests."],"assumptions":[],"uncertainty":[],"affected_files":["src/validate.sh"],"affected_tasks":["validate"],"supersedes":null}]);
            }
            "deferrable" => {
                let mut plan: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(root.join("plan.json"))?)?;
                plan["human_requirements"] = json!([{"kind":"environment","reason":"Validation needs a developer-owned fixture","requested_action":"Provide the exact approved validation fixture externally","affected_tasks":["validate"],"blocking":true,"evidence":"Deterministic task-scoped fixture hold","confidence":100,"permission":null}]);
                std::fs::write(root.join("plan.json"), serde_json::to_vec(&plan)?)?;
            }
            "command" => {
                coder["command"] = json!(["/bin/sh", "-c", "echo forbidden"]);
            }
            _ => {
                let path = match scenario {
                    "outside" => "README.md",
                    "credential" => "src/.env",
                    "configuration" => "src/package.json",
                    _ => "src/../../escape.sh",
                };
                coder["edits"]
                    .as_array_mut()
                    .context("edits")?
                    .push(json!({"path":path,"content":"forbidden"}));
            }
        }
        std::fs::write(root.join("task-validate.json"), serde_json::to_vec(&coder)?)?;
        let mut daemon = start(&home, &root)?;
        let m = ready(&home, None).await?;
        let http = reqwest::Client::builder().no_proxy().build()?;
        let id = Uuid::new_v4();
        let request = DelegationProposalRequest {
            submission: PlanSubmission {
                id,
                repository: repo.clone(),
                base: "main".into(),
                specification:
                    "Add CSV export to the reporting module, including validation and tests.".into(),
            },
            source_scope: Some(SourceScope::default()),
            writable_files: vec![],
            max_runtime_seconds: 120,
            local_integration: true,
        };
        let contract: DelegationContract = http
            .post(format!("http://{}/v1/delegations/propose", m.address))
            .bearer_auth(&m.token)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut widened = contract.clone();
        widened
            .request
            .source_scope
            .as_mut()
            .context("scope")?
            .write_subtrees
            .push("infra".into());
        assert!(
            http.post(format!(
                "http://{}/v1/delegations/{id}/authorize",
                m.address
            ))
            .bearer_auth(&m.token)
            .json(&widened)
            .send()
            .await?
            .status()
            .is_client_error()
        );
        assert!(!root.join("invocations").exists());
        http.post(format!(
            "http://{}/v1/delegations/{id}/authorize",
            m.address
        ))
        .bearer_auth(&m.token)
        .json(&contract)
        .send()
        .await?
        .error_for_status()?;
        let run = drained(&http, &m, id).await?;
        assert_eq!(
            run.status,
            EngineeringStatus::Blocked,
            "{scenario}: {} {:?}",
            run.summary,
            run.tasks
        );
        assert!(
            run.tasks
                .iter()
                .any(|t| t.plan.id == "format" && t.status == TaskStatus::Completed),
            "{scenario}: independent task lost"
        );
        assert!(
            run.handoff
                .as_ref()
                .is_some_and(|h| h.outcome == HandoffOutcome::PartiallyCompleted
                    && !h.completion_requirements_satisfied),
            "{scenario}"
        );
        let task = run
            .tasks
            .iter()
            .find(|t| t.plan.id == "validate")
            .context("validate")?;
        if let Some(w) = &task.workspace {
            if scenario == "ignored" {
                assert!(
                    w.path.join("src/validate.sh").exists(),
                    "the ignored candidate must be retained for inspection"
                );
                assert!(
                    task.attempts
                        .iter()
                        .all(|a| a.verification.is_empty() && a.reviews.is_empty()),
                    "an omitted source edit must block before any verification/review receipt"
                );
                assert!(task.artifact_commit.is_none());
            } else {
                assert!(
                    !w.path.join("src/validate.sh").exists(),
                    "{scenario}: invalid batch applied"
                );
            }
        }
        assert!(!repo.join("src/validate.sh").exists());
        assert_eq!(run.delegation.as_ref().context("auth")?.contract, contract);
        evidence.push(json!({"scenario":scenario,"run":run}));
        daemon.kill().await?;
    }
    if let Some(destination) = std::env::var_os("AUTONO_BUILD_DENIALS_EVIDENCE") {
        std::fs::write(destination, serde_json::to_vec_pretty(&evidence)?)?;
    }
    Ok(())
}
