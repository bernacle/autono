//! A4 real daemon/SQLite/worktrees/checks/fresh reviewer; shell providers, zero inference.
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

fn install(root: &Path, runtime: bool) -> Result<()> {
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
 test "$arg" != --resume; test "$arg" != --add-dir
 prompt=$arg
done
test "$saw" = 1
label=${PWD##*/}
printf '%s\n' "$label" >> "$AUTONO_FIXTURE_ROOT/invocations"
case "$label" in
 planner)
  if test -f "$AUTONO_FIXTURE_ROOT/hold-planner"; then
   touch "$AUTONO_FIXTURE_ROOT/planner-started"
   while test -f "$AUTONO_FIXTURE_ROOT/hold-planner"; do sleep 0.01; done
  fi
  if test -f "$AUTONO_FIXTURE_ROOT/fail-planner"; then exit 42; fi
  result=$(cat "$AUTONO_FIXTURE_ROOT/plan.json");;
 task-D)
  if test -f "$AUTONO_FIXTURE_ROOT/runtime" && ! test -f "$AUTONO_FIXTURE_ROOT/access-confirmed"; then
   if test -f "$AUTONO_FIXTURE_ROOT/first-candidate" && ! test -f "$AUTONO_FIXTURE_ROOT/candidate-used"; then
    touch "$AUTONO_FIXTURE_ROOT/candidate-used"
    result=$(cat "$AUTONO_FIXTURE_ROOT/task-D.json")
   else
    result='{"summary":"Access required","blocker":"Configure the fixture external test account through its existing mechanism; never submit credentials","decisions":[],"edits":[]}'
   fi
  else
   case "$prompt" in *'Fixture test account externally configured'*) : ;; *) exit 89;; esac
   result=$(cat "$AUTONO_FIXTURE_ROOT/task-D.json")
  fi;;
 task-*) result=$(cat "$AUTONO_FIXTURE_ROOT/$label.json");;
 review-*)
  id=${label#review-}; id=${id%-*}
  case "$prompt" in *'Original delegated feature specification:'*'Local account simulator is explicitly permitted'*) : ;; *) exit 91;; esac
  case "$prompt" in *'"diff_excerpt"'*'"verification"'*) : ;; *) exit 92;; esac
  case "$prompt" in *'"candidate_tree"'*|*'"verified_tree"'*) : ;; *) exit 93;; esac
  result="{\"accepted\":true,\"criteria\":[{\"criterion_id\":\"$id-C1\",\"criterion\":\"Works\",\"satisfied\":true,\"evidence\":\"Inspected original fixture contract, full candidate diff and actual sandboxed check receipts. This verifies the local simulator, not a real external service.\"}],\"findings\":[],\"blocker\":\"\"}";;
 *) exit 94;;
esac
uuid=$(if test -f /proc/sys/kernel/random/uuid; then cat /proc/sys/kernel/random/uuid; else /usr/bin/uuidgen; fi)
printf '{"type":"result","subtype":"success","session_id":"%s","structured_output":%s}\n' "$uuid" "$result"
"##;
    std::fs::write(root.join("bin/claude"), script)?;
    std::fs::set_permissions(
        root.join("bin/claude"),
        std::fs::Permissions::from_mode(0o755),
    )?;
    if runtime {
        std::fs::write(root.join("runtime"), "discover at task D")?;
    }
    let ids = ["A", "B", "C", "D", "E"];
    let tasks: Vec<_> = ids.iter().map(|id| json!({"id":id,"goal":format!("Feature {id}"),"description":format!("Implement {id}"),"acceptance_criteria":["Behavior satisfies the explicit local simulator contract"],"dependencies":match *id {"B"|"C"=>vec!["A"],"D"=>vec!["B","C"],"E"=>vec!["C"],_=>vec![]},"requirement_ids":["feature"],"verification_expectations":["Configured sandboxed checks and fresh independent review"],"role":"coder"})).collect();
    let requirement = json!({"kind":"environment","reason":"Fixture external test account is unavailable","requested_action":"Configure the fixture external test account through its existing mechanism; never submit credentials","affected_tasks":["D"],"blocking":true,"evidence":"Required only by task D; local implementation and tests do not use the account","confidence":100,"permission":null});
    std::fs::write(
        root.join("plan.json"),
        serde_json::to_vec(
            &json!({"decisions":[],"human_requirements":if runtime {vec![]} else {vec![requirement]},"requirements":[{"id":"feature","text":"Local model, API, validation, external adapter and independent tests"}],"milestones":[{"id":"feature","goal":"Feature","task_ids":ids}],"tasks":tasks}),
        )?,
    )?;
    for (id, file, content) in [
        ("A", "model.sh", "model() { printf '%s\\n' \"$1\"; }\n"),
        ("B", "api.sh", ". ./model.sh\napi() { model \"$1\"; }\n"),
        ("C", "validate.sh", "validate() { test -n \"$1\"; }\n"),
        (
            "D",
            "external.sh",
            ". ./api.sh\n. ./validate.sh\n# Explicit local account simulator, no network or real service claim.\nexternal() { validate \"$1\" && api \"$1\"; }\n",
        ),
        (
            "E",
            "test.sh",
            ". ./model.sh\n. ./validate.sh\ntest \"$(model example)\" = example\nvalidate example\nif validate ''; then exit 1; fi\n",
        ),
    ] {
        std::fs::write(
            root.join(format!("task-{id}.json")),
            serde_json::to_vec(
                &json!({"summary":"Implemented local fixture contract","blocker":"","decisions":[],"edits":[{"path":file,"content":content}]}),
            )?,
        )?;
    }
    Ok(())
}
struct Harness {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    repo: PathBuf,
}
impl Harness {
    async fn new(runtime: bool) -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let root = std::fs::canonicalize(temp.path())?;
        let home = root.join("home");
        let repo = root.join("repo");
        std::fs::create_dir(&home)?;
        std::fs::create_dir(&repo)?;
        install(&root, runtime)?;
        std::fs::write(
            home.join("config.toml"),
            r#"
[runner]
keep_awake = false
[agents.planner]
provider = "claude"
model = "fixture"
timeout_seconds = 15
[agents.coder]
provider = "claude"
model = "fixture"
timeout_seconds = 15
[agents.reviewer]
provider = "claude"
model = "fixture"
timeout_seconds = 15
[verification]
commands = [["/bin/sh", "-c", 'set -eu; for f in model.sh api.sh validate.sh external.sh test.sh; do if test -f "$f"; then /bin/sh -n "$f"; fi; done; if test -f model.sh; then . ./model.sh; test "$(model item)" = item; fi; if test -f api.sh; then . ./api.sh; test "$(api item)" = item; fi; if test -f validate.sh; then . ./validate.sh; validate item; if validate ""; then exit 1; fi; fi; if test -f test.sh; then /bin/sh -eu ./test.sh; fi; if test -f external.sh; then . ./external.sh; test "$(external item)" = item; if external ""; then exit 1; fi; fi; if test "${PWD##*/}" = integration; then test -f external.sh && test -f test.sh; fi']]
timeout_seconds = 10
[permissions.rules]
workspace_write = "REQUIRE_APPROVAL"
run_checks = "REQUIRE_APPROVAL"
"#,
        )?;
        git(&repo, &["init", "-b", "main"]).await?;
        std::fs::write(repo.join("README.md"), "Local simulator fixture\n")?;
        git(&repo, &["add", "."]).await?;
        git(&repo, &["commit", "-m", "seed"]).await?;
        Ok(Self {
            _temp: temp,
            root,
            home,
            repo,
        })
    }
    async fn propose(
        &self,
        http: &reqwest::Client,
        m: &autono_protocol::DaemonMetadata,
        seconds: u64,
    ) -> Result<DelegationContract> {
        let request = DelegationProposalRequest {submission:PlanSubmission {id:Uuid::new_v4(),repository:self.repo.clone(),base:"main".into(),specification:"Build model, API, validation, external adapter and independent tests. Local account simulator is explicitly permitted for this deterministic fixture; this does not verify a real service. Access is required only for task D.".into()},writable_files:["model.sh","api.sh","validate.sh","external.sh","test.sh"].iter().map(|s| (*s).to_string()).collect(),max_runtime_seconds:seconds,local_integration:true};
        Ok(http
            .post(format!("http://{}/v1/delegations/propose", m.address))
            .bearer_auth(&m.token)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}
#[tokio::test]
async fn five_tasks_continue_while_planned_or_runtime_access_waits_then_resume_without_new_consent()
-> Result<()> {
    let mut evidence = vec![];
    for runtime in [false, true] {
        let h = Harness::new(runtime).await?;
        let original_main = git(&h.repo, &["rev-parse", "main"]).await?;
        let mut daemon = start(&h.home, &h.root)?;
        let m = ready(&h.home, None).await?;
        let http = reqwest::Client::builder().no_proxy().build()?;
        let c = h.propose(&http, &m, 120).await?;
        let id = c.request.submission.id;
        http.post(format!(
            "http://{}/v1/delegations/{id}/authorize",
            m.address
        ))
        .bearer_auth(&m.token)
        .json(&c)
        .send()
        .await?
        .error_for_status()?;
        let partial = drained(&http, &m, id).await?;
        assert_eq!(
            partial.status,
            EngineeringStatus::Blocked,
            "{}",
            partial.summary
        );
        for t in &partial.tasks {
            if t.plan.id == "D" {
                assert_ne!(t.status, TaskStatus::Completed);
                if !runtime {
                    assert!(t.attempts.is_empty());
                }
            } else {
                assert_eq!(
                    t.status,
                    TaskStatus::Completed,
                    "{}: {:?}",
                    t.plan.id,
                    t.attempts
                );
            }
        }
        let report = partial.handoff.as_ref().context("handoff")?;
        assert_eq!(report.outcome, HandoffOutcome::PartiallyCompleted);
        assert_eq!(
            report
                .tasks
                .iter()
                .filter(|t| t.delivery == TaskDelivery::IndependentlyVerifiedLocally)
                .count(),
            4
        );
        assert!(!report.completion_requirements_satisfied && !report.combined_checks_passed);
        assert_eq!(report.needs_you.len(), 1);
        let action = partial
            .human_actions
            .iter()
            .find(|a| partial.action_blocks(a))
            .context("action")?;
        assert_eq!(
            action
                .prerequisite
                .as_ref()
                .context("classification")?
                .category,
            PrerequisiteCategory::DeferrableManualDependency
        );
        let url = format!("http://{}/v1/engineering/{id}/handoff", m.address);
        assert_eq!(
            http.get(&url).send().await?.status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        let api: RunHandoff = http
            .get(&url)
            .bearer_auth(&m.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(&api, report);
        let cli_report = invoke(&h.home, &["handoff", &id.to_string(), "--json"]).await?;
        assert_eq!(serde_json::from_str::<RunHandoff>(&cli_report)?, api);
        assert!(
            invoke(&h.home, &["handoff", &id.to_string()])
                .await?
                .contains("Needs you:")
        );
        // Reopen the durable SQLite state with no process replay and no desktop.
        let calls = std::fs::read_to_string(h.root.join("invocations"))?;
        daemon.kill().await?;
        daemon = start(&h.home, &h.root)?;
        let restarted = ready(&h.home, Some(m.pid)).await?;
        let restored = run_view(&http, &restarted, id).await?;
        assert_eq!(restored.handoff, partial.handoff);
        assert_eq!(std::fs::read_to_string(h.root.join("invocations"))?, calls);
        let resolve_url = format!(
            "http://{}/v1/engineering/{id}/blockers/{}/resolve",
            restarted.address, action.id
        );
        let mut request = ResolutionRequest {
            expected_action_version: Some(action.audit.len() + 1),
            request_id: Uuid::new_v4(),
            disposition: ResolutionDisposition::Resolve,
            note: "Fixture test account externally configured".into(),
            effect: EffectState::NotPerformed,
            permission: None,
        };
        assert_eq!(
            http.post(&resolve_url)
                .bearer_auth(&restarted.token)
                .json(&request)
                .send()
                .await?
                .status(),
            reqwest::StatusCode::CONFLICT
        );
        request.expected_action_version = Some(action.audit.len());
        std::fs::write(h.root.join("access-confirmed"), "fixture only")?;
        let receipt: EngineeringRun = http
            .post(&resolve_url)
            .bearer_auth(&restarted.token)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if run_view(&http, &restarted, id).await?.revision > receipt.revision {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        let complete = drained(&http, &restarted, id).await?;
        assert_eq!(
            complete.status,
            EngineeringStatus::Completed,
            "{} {:?}",
            complete.summary,
            complete.human_actions
        );
        assert_eq!(complete.delegation, partial.delegation);
        for t in partial.tasks.iter().filter(|t| t.plan.id != "D") {
            assert_eq!(
                serde_json::to_value(t)?,
                serde_json::to_value(complete.tasks.iter().find(|x| x.plan.id == t.plan.id))?
            );
        }
        assert!(
            complete
                .handoff
                .as_ref()
                .context("complete handoff")?
                .completion_requirements_satisfied
        );
        let replay: EngineeringRun = http
            .post(&resolve_url)
            .bearer_auth(&restarted.token)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(replay.revision, receipt.revision);
        let sql = autono_core::Store::open(&h.home.join("runtime.sqlite3")).await?;
        assert_eq!(sql.engineering_get(id).await?.handoff, complete.handoff);
        assert_eq!(git(&h.repo, &["rev-parse", "main"]).await?, original_main);
        let events: Vec<EngineeringEvent> = http
            .get(format!(
                "http://{}/v1/engineering/{id}/events?limit=500",
                restarted.address
            ))
            .bearer_auth(&restarted.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert!(!events.iter().any(|e| matches!(
            e.kind.as_str(),
            "approved" | "continued" | "integration_requested"
        )));
        evidence.push(json!({"runtime_discovery":runtime,"partial":partial,"completed":complete,"events":events,"original_main":original_main.trim(),"no_process_replay_on_restart":true,"real_service_verified":false,"paid_inference":false}));
        daemon.kill().await?;
    }
    if let Some(path) = std::env::var_os("AUTONO_PREREQUISITES_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence)?)?;
    }
    Ok(())
}

#[tokio::test]
async fn missing_essential_authentication_stops_before_planning_and_cannot_be_waived() -> Result<()>
{
    let h = Harness::new(false).await?;
    let mut command = cli(&h.home);
    command
        .arg("daemon")
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", h.root.join("bin").display()),
        )
        .env("AUTONO_FIXTURE_ROOT", &h.root)
        .env_remove("ANTHROPIC_API_KEY");
    let mut daemon = command.spawn()?;
    let m = ready(&h.home, None).await?;
    let http = reqwest::Client::builder().no_proxy().build()?;
    let c = h.propose(&http, &m, 120).await?;
    let id = c.request.submission.id;
    http.post(format!(
        "http://{}/v1/delegations/{id}/authorize",
        m.address
    ))
    .bearer_auth(&m.token)
    .json(&c)
    .send()
    .await?
    .error_for_status()?;
    let held = drained(&http, &m, id).await?;
    assert!(held.plan.is_none() && held.planner.is_none());
    assert!(!h.root.join("invocations").exists());
    let check = held
        .preflight
        .as_ref()
        .context("preflight")?
        .checks
        .iter()
        .find(|c| c.id == "authentication-bounded-api")
        .context("auth check")?;
    assert_eq!(check.ready, Some(false));
    let a = held
        .human_actions
        .iter()
        .find(|a| a.requirement.reason.contains("ANTHROPIC_API_KEY"))
        .context("auth action")?;
    assert_eq!(
        a.prerequisite.as_ref().context("category")?.category,
        PrerequisiteCategory::EssentialBeforeStart
    );
    let q = ResolutionRequest {
        expected_action_version: Some(a.audit.len()),
        request_id: Uuid::new_v4(),
        disposition: ResolutionDisposition::Resolve,
        note: "Acknowledged; this cannot repair daemon environment".into(),
        effect: EffectState::NotPerformed,
        permission: None,
    };
    http.post(format!(
        "http://{}/v1/engineering/{id}/blockers/{}/resolve",
        m.address, a.id
    ))
    .bearer_auth(&m.token)
    .json(&q)
    .send()
    .await?
    .error_for_status()?;
    let still = drained(&http, &m, id).await?;
    assert!(still.plan.is_none());
    assert!(!still.preflight.as_ref().context("report")?.ready);
    assert!(!h.root.join("invocations").exists());
    // Repair externally by restarting with the fixture credential. Refresh can admit
    // the never-invoked planner, but cannot replay an interrupted planner.
    daemon.kill().await?;
    daemon = start(&h.home, &h.root)?;
    let restarted = ready(&h.home, Some(m.pid)).await?;
    invoke(&h.home, &["preflight", &id.to_string()]).await?;
    let planned = drained(&http, &restarted, id).await?;
    assert!(planned.plan.is_some(), "{}", planned.summary);
    assert_eq!(
        std::fs::read_to_string(h.root.join("invocations"))?
            .lines()
            .filter(|l| *l == "planner")
            .count(),
        1
    );
    daemon.kill().await?;
    Ok(())
}

#[tokio::test]
async fn shared_multiple_dependent_and_failed_branches_keep_truthful_scopes() -> Result<()> {
    for case in ["shared", "dependent", "multiple", "failed"] {
        let h = Harness::new(false).await?;
        let path = h.root.join("plan.json");
        let mut p: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let mut need = p["human_requirements"][0].clone();
        match case {
            "shared" => {
                need["affected_tasks"] = json!(["B"]);
                p["human_requirements"]
                    .as_array_mut()
                    .context("requirements")?
                    .push(need);
            }
            "dependent" => {
                p["human_requirements"][0]["affected_tasks"] = json!(["B"]);
            }
            "multiple" => {
                need["reason"] = json!("A separate validation choice is required");
                need["requested_action"] = json!("Confirm the validation contract");
                need["affected_tasks"] = json!(["C"]);
                p["human_requirements"]
                    .as_array_mut()
                    .context("requirements")?
                    .push(need);
            }
            "failed" => {
                std::fs::write(
                    h.root.join("task-E.json"),
                    serde_json::to_vec(
                        &json!({"summary":"Candidate with failing local tests","blocker":"","decisions":[],"edits":[{"path":"test.sh","content":"exit 1\n"}]}),
                    )?,
                )?;
            }
            _ => unreachable!(),
        }
        std::fs::write(path, serde_json::to_vec(&p)?)?;
        let mut daemon = start(&h.home, &h.root)?;
        let m = ready(&h.home, None).await?;
        let http = reqwest::Client::builder().no_proxy().build()?;
        let c = h.propose(&http, &m, 120).await?;
        let id = c.request.submission.id;
        http.post(format!(
            "http://{}/v1/delegations/{id}/authorize",
            m.address
        ))
        .bearer_auth(&m.token)
        .json(&c)
        .send()
        .await?
        .error_for_status()?;
        let run = drained(&http, &m, id).await?;
        assert_eq!(
            run.status,
            EngineeringStatus::Blocked,
            "{case}: {}",
            run.summary
        );
        let report = run.handoff.as_ref().context("handoff")?;
        assert!(!report.completion_requirements_satisfied);
        assert_eq!(
            report.needs_you.len(),
            if case == "multiple" { 2 } else { 1 }
        );
        match case {
            "shared" => assert_eq!(report.needs_you[0].affected_tasks, vec!["B", "D"]),
            "dependent" => {
                assert_eq!(report.needs_you[0].affected_tasks, vec!["B"]);
                assert_eq!(report.needs_you[0].waiting_descendants, vec!["D"]);
                assert_eq!(
                    run.tasks
                        .iter()
                        .find(|t| t.plan.id == "D")
                        .context("D")?
                        .status,
                    TaskStatus::Waiting
                );
                assert_eq!(
                    run.tasks
                        .iter()
                        .find(|t| t.plan.id == "E")
                        .context("E")?
                        .status,
                    TaskStatus::Completed
                );
            }
            "multiple" => {
                assert_eq!(
                    run.tasks
                        .iter()
                        .find(|t| t.plan.id == "B")
                        .context("B")?
                        .status,
                    TaskStatus::Completed
                );
                assert_eq!(
                    run.tasks
                        .iter()
                        .find(|t| t.plan.id == "E")
                        .context("E")?
                        .status,
                    TaskStatus::Waiting
                );
            }
            "failed" => {
                let e = report
                    .tasks
                    .iter()
                    .find(|t| t.task_id == "E")
                    .context("E")?;
                assert_eq!(e.delivery, TaskDelivery::Failed);
                assert!(e.implementation_recorded && !e.checks_passed);
                let task = run.tasks.iter().find(|t| t.plan.id == "E").context("E")?;
                assert_eq!(task.attempts.len(), run.settings.execution.max_attempts);
                assert!(task.attempts.iter().all(|a| a.reviews.is_empty()));
            }
            _ => unreachable!(),
        }
        // Cancellation is authoritative even for a fresh, correctly versioned resolution.
        http.post(format!("http://{}/v1/engineering/{id}/cancel", m.address))
            .bearer_auth(&m.token)
            .json(&json!({}))
            .send()
            .await?
            .error_for_status()?;
        let a = &run.human_actions[0];
        let resolution = ResolutionRequest {
            expected_action_version: Some(a.audit.len()),
            request_id: Uuid::new_v4(),
            disposition: ResolutionDisposition::Resolve,
            note: "Too late; retain cancellation".into(),
            effect: EffectState::NotPerformed,
            permission: None,
        };
        assert_eq!(
            http.post(format!(
                "http://{}/v1/engineering/{id}/blockers/{}/resolve",
                m.address, a.id
            ))
            .bearer_auth(&m.token)
            .json(&resolution)
            .send()
            .await?
            .status(),
            reqwest::StatusCode::CONFLICT
        );
        daemon.kill().await?;
    }
    Ok(())
}

#[tokio::test]
async fn resolution_after_original_deadline_records_no_new_authority_or_inference() -> Result<()> {
    let h = Harness::new(false).await?;
    let mut cmd = cli(&h.home);
    cmd.arg("daemon")
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", h.root.join("bin").display()),
        )
        .env("AUTONO_FIXTURE_ROOT", &h.root)
        .env_remove("ANTHROPIC_API_KEY");
    let mut daemon = cmd.spawn()?;
    let m = ready(&h.home, None).await?;
    let http = reqwest::Client::builder().no_proxy().build()?;
    let c = h.propose(&http, &m, 5).await?;
    let id = c.request.submission.id;
    http.post(format!(
        "http://{}/v1/delegations/{id}/authorize",
        m.address
    ))
    .bearer_auth(&m.token)
    .json(&c)
    .send()
    .await?
    .error_for_status()?;
    let run = drained(&http, &m, id).await?;
    let a = run.human_actions.first().context("prerequisite")?;
    let auth = run.delegation.as_ref().context("delegation")?;
    tokio::time::sleep(
        (auth.expires_at - chrono::Utc::now())
            .to_std()
            .unwrap_or_default()
            + Duration::from_millis(20),
    )
    .await;
    let q = ResolutionRequest {
        expected_action_version: Some(a.audit.len()),
        request_id: Uuid::new_v4(),
        disposition: ResolutionDisposition::Resolve,
        note: "Fixture setup acknowledged after original grant expiry".into(),
        effect: EffectState::NotPerformed,
        permission: None,
    };
    http.post(format!(
        "http://{}/v1/engineering/{id}/blockers/{}/resolve",
        m.address, a.id
    ))
    .bearer_auth(&m.token)
    .json(&q)
    .send()
    .await?
    .error_for_status()?;
    let after = drained(&http, &m, id).await?;
    assert_eq!(after.delegation, run.delegation);
    assert!(after.plan.is_none() && !h.root.join("invocations").exists());
    assert!(
        !after
            .handoff
            .context("handoff")?
            .completion_requirements_satisfied
    );
    daemon.kill().await?;
    Ok(())
}

#[tokio::test]
async fn retained_candidate_survives_a_failed_attempt_then_an_empty_blocker_response() -> Result<()>
{
    let h = Harness::new(true).await?;
    std::fs::write(
        h.root.join("first-candidate"),
        "one admitted local candidate",
    )?;
    std::fs::write(
        h.root.join("task-D.json"),
        serde_json::to_vec(
            &json!({"summary":"Candidate awaiting correction","blocker":"","decisions":[],"edits":[{"path":"external.sh","content":"exit 1\n"}]}),
        )?,
    )?;
    let mut daemon = start(&h.home, &h.root)?;
    let m = ready(&h.home, None).await?;
    let http = reqwest::Client::builder().no_proxy().build()?;
    let c = h.propose(&http, &m, 120).await?;
    let id = c.request.submission.id;
    http.post(format!(
        "http://{}/v1/delegations/{id}/authorize",
        m.address
    ))
    .bearer_auth(&m.token)
    .json(&c)
    .send()
    .await?
    .error_for_status()?;
    let run = drained(&http, &m, id).await?;
    assert_eq!(run.status, EngineeringStatus::Blocked, "{}", run.summary);
    let t = run.tasks.iter().find(|t| t.plan.id == "D").context("D")?;
    assert_eq!(t.attempts.len(), 2);
    let report = run.handoff.context("handoff")?;
    let d = report
        .tasks
        .iter()
        .find(|t| t.task_id == "D")
        .context("D report")?;
    assert!(d.implementation_recorded && !d.checks_passed);
    assert_eq!(d.delivery, TaskDelivery::ImplementedNotFullyVerified);
    assert_eq!(
        t.attempts[0].implementation_tree,
        t.attempts[1].implementation_tree
    );
    daemon.kill().await?;
    Ok(())
}

#[tokio::test]
async fn repaired_preplanning_admission_remains_owned_on_failure_or_restart() -> Result<()> {
    for interrupted in [false, true] {
        let h = Harness::new(false).await?;
        let mut cmd = cli(&h.home);
        cmd.arg("daemon")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", h.root.join("bin").display()),
            )
            .env("AUTONO_FIXTURE_ROOT", &h.root)
            .env_remove("ANTHROPIC_API_KEY");
        let mut daemon = cmd.spawn()?;
        let m = ready(&h.home, None).await?;
        let http = reqwest::Client::builder().no_proxy().build()?;
        let c = h.propose(&http, &m, 120).await?;
        let id = c.request.submission.id;
        http.post(format!(
            "http://{}/v1/delegations/{id}/authorize",
            m.address
        ))
        .bearer_auth(&m.token)
        .json(&c)
        .send()
        .await?
        .error_for_status()?;
        let held = drained(&http, &m, id).await?;
        assert!(held.planner.is_none());
        daemon.kill().await?;
        daemon = start(&h.home, &h.root)?;
        let repaired = ready(&h.home, Some(m.pid)).await?;
        std::fs::write(
            h.root.join(if interrupted {
                "hold-planner"
            } else {
                "fail-planner"
            }),
            "fixture boundary",
        )?;
        invoke(&h.home, &["preflight", &id.to_string()]).await?;
        let final_run = if interrupted {
            tokio::time::timeout(Duration::from_secs(15), async {
                while !h.root.join("planner-started").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await?;
            assert_eq!(
                run_view(&http, &repaired, id).await?.status,
                EngineeringStatus::Planning
            );
            daemon.kill().await?;
            std::fs::remove_file(h.root.join("hold-planner"))?;
            daemon = start(&h.home, &h.root)?;
            let restarted = ready(&h.home, Some(repaired.pid)).await?;
            let r = run_view(&http, &restarted, id).await?;
            assert_eq!(r.status, EngineeringStatus::Interrupted);
            r
        } else {
            let r = drained(&http, &repaired, id).await?;
            assert_eq!(r.status, EngineeringStatus::Failed, "{}", r.summary);
            r
        };
        assert!(final_run.plan.is_none() && final_run.planner.is_some());
        assert_eq!(
            std::fs::read_to_string(h.root.join("invocations"))?
                .lines()
                .filter(|l| *l == "planner")
                .count(),
            1
        );
        daemon.kill().await?;
    }
    Ok(())
}
