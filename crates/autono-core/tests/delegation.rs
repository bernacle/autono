#![cfg(any(target_os = "macos", target_os = "linux"))]
use anyhow::{Context, Result, ensure};
use autono_core::{
    Store, Supervisor,
    config::Config,
    engineering::{
        agent::{AgentFuture, AgentRequest, AgentResponse, AgentRuntime, CliAgentRuntime},
        process::{CommandOutput, ProcessRunner},
        *,
    },
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Case {
    Success,
    DecisionNoContext,
    DecisionConflict,
    DecisionOutside,
    DecisionExternal,
    DecisionArchitecture,
    DecisionRetry,
    DecisionReview,
    DecisionReplay,
    DecisionBad,
    DecisionSecret,
    DecisionContradictory,
    DecisionWrongEvidence,
    DecisionSmuggle,
    DecisionUnknown,
    DecisionUncertain,
    UnauthorizedCommand,
    OutsideFile,
    UnknownEffect,
    Login,
    Slow,
    Drift,
    Human,
    Unsupported,
    ScopedPermission,
    LiveHuman,
    PausePlanner,
    PauseReviewer,
    PreflightHold,
    PauseVerification,
    PauseIntegration,
}
struct Fixture {
    case: Case,
    calls: Mutex<BTreeMap<String, usize>>,
    reached: tokio::sync::Notify,
    release: CancellationToken,
    delayed_readiness: std::sync::atomic::AtomicBool,
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
impl AgentRuntime for Fixture {
    fn readiness<'a>(
        &'a self,
        runner: &'a ProcessRunner,
        cwd: &'a std::path::Path,
        _: &'a WorkflowSettings,
        stop: &'a CancellationToken,
    ) -> autono_core::engineering::agent::ReadinessFuture<'a> {
        Box::pin(async move {
            if self
                .delayed_readiness
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                let gate = cwd.join("release-probe");
                let command = vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    format!(
                        "while test ! -f {}; do sleep 0.01; done; echo capability",
                        quote(&gate.to_string_lossy())
                    ),
                ];
                let process = runner.start(command, cwd, 15, Uuid::new_v4(), stop).await?;
                self.reached.notify_one();
                let _ = runner.wait(process.id, stop).await?;
                // Deliberately attempt the next probe even if the first was stopped.
                // Production adapters also recheck here; the runner must deny it.
                runner
                    .run(
                        vec![
                            "/bin/sh".into(),
                            "-c".into(),
                            "echo forbidden > after-deadline".into(),
                        ],
                        cwd,
                        15,
                        Uuid::new_v4(),
                        stop,
                    )
                    .await?;
            }
            Ok(vec![ReadinessCheck {
                id: "fixture-readiness".into(),
                ready: Some(true),
                blocking: true,
                evidence: "Deterministic local capability fixture".into(),
            }])
        })
    }
    fn prepare<'a>(
        &'a self,
        r: &'a AgentRequest,
        _: &'a ProcessRunner,
        stop: &'a CancellationToken,
    ) -> AgentFuture<'a> {
        Box::pin(async move {
            ensure!(r.bounded, "fixture requires bounded mode");
            ensure!(r.resume.is_none(), "autonomous sessions must be fresh");
            if r.role == AgentRole::Coder {
                ensure!(
                    r.prompt.contains("Task-relevant requirements:")
                        && r.prompt.contains("Criteria with stable IDs:"),
                    "fresh autonomous retries must receive the complete authoritative task context"
                );
            }
            if r.role == AgentRole::Reviewer {
                ensure!(
                    r.prompt
                        .contains("Original delegated feature specification:"),
                    "independent autonomous review must retain the developer's original intent"
                );
            }
            let label = r
                .cwd
                .file_name()
                .context("label")?
                .to_string_lossy()
                .into_owned();
            let call = {
                let mut calls = self.calls.lock().expect("calls");
                let v = calls.entry(label.clone()).or_default();
                *v += 1;
                *v
            };
            if (self.case == Case::PausePlanner && r.role == AgentRole::Planner)
                || (self.case == Case::PauseReviewer && r.role == AgentRole::Reviewer)
                || (self.case == Case::Drift && r.role == AgentRole::Coder)
                || (self.case == Case::LiveHuman
                    && r.role == AgentRole::Reviewer
                    && label.starts_with("review-a-"))
            {
                self.reached.notify_one();
                tokio::select! { _ = self.release.cancelled() => {}, _ = stop.cancelled() => anyhow::bail!("stopped") }
            }
            let id = if label.contains("task-") {
                label.trim_start_matches("task-").to_string()
            } else if label.starts_with("review-") {
                label.split('-').nth(1).unwrap_or("a").to_string()
            } else {
                "a".into()
            };
            let value = match r.role {
                AgentRole::Planner => {
                    json!({"human_requirements":[],"requirements":[{"id":"r","text":"Feature"}],"milestones":[{"id":"m","goal":"Feature","task_ids":["a","b","c"]}],"tasks":[
                    {"id":"a","goal":"A","description":"Create A","acceptance_criteria":["A works"],"dependencies":[],"requirement_ids":["r"],"verification_expectations":["Configured checks"],"role":"coder"},
                    {"id":"b","goal":"B","description":"Create B","acceptance_criteria":["B works"],"dependencies":[],"requirement_ids":["r"],"verification_expectations":["Configured checks"],"role":"coder"},
                    {"id":"c","goal":"C","description":"Create C","acceptance_criteria":["C works"],"dependencies":["a"],"requirement_ids":["r"],"verification_expectations":["Configured checks"],"role":"coder"}]})
                }
                AgentRole::Coder => match self.case {
                    Case::UnauthorizedCommand if id == "b" => {
                        json!({"summary":"Run shell","blocker":"","command":["sh","-c","touch /tmp/autono-unauthorized"],"edits":[]})
                    }
                    Case::OutsideFile if id == "b" => {
                        json!({"summary":"Outside","blocker":"","edits":[{"path":"b.txt","content":"must not be applied"},{"path":"../escape.txt","content":"bad"}]})
                    }
                    Case::UnknownEffect if id == "b" => {
                        json!({"summary":"Unknown external effect","blocker":"Inspect whether the remote side effect happened; its outcome is unknown","edits":[]})
                    }
                    Case::Human | Case::LiveHuman if id == "b" && call == 1 => {
                        json!({"summary":"Decision","blocker":"Choose the local feature behavior","edits":[]})
                    }
                    _ => {
                        json!({"summary":"Implemented","blocker":"","edits":[{"path":format!("{id}.txt"),"content": if self.case == Case::Success && id == "b" && call == 1 {"bad\n"} else {"good\n"}}]})
                    }
                },
                AgentRole::Reviewer => {
                    json!({"accepted":true,"criteria":[{"criterion_id":format!("{id}-C1"),"criterion":"Works","satisfied":true,"evidence":"Independent inspection of full candidate diff and verification"}],"findings":[],"blocker":""})
                }
            };
            let mut value = value;
            if self.case == Case::ScopedPermission && r.role == AgentRole::Planner {
                value["human_requirements"] = json!([{"kind":"permission","reason":"B requests an operation beyond the configured check grant","requested_action":"Approve an unconfigured shell command","affected_tasks":["b"],"blocking":true,"evidence":"Fixture proposes a mismatched verification scope","confidence":100,"permission":{"operation":"run_checks","scope":"arbitrary agent-generated shell command"}}]);
            }
            if self.case == Case::PreflightHold && r.role == AgentRole::Planner {
                value["human_requirements"] = json!([{"kind":"product_decision","reason":"Fixture hold before execution","requested_action":"Choose behavior","affected_tasks":[],"blocking":true,"evidence":"Deterministic preparation fixture","confidence":100,"permission":null}]);
            }
            let decision_case = matches!(
                self.case,
                Case::DecisionNoContext
                    | Case::DecisionConflict
                    | Case::DecisionOutside
                    | Case::DecisionExternal
                    | Case::DecisionArchitecture
                    | Case::DecisionRetry
                    | Case::DecisionReview
                    | Case::DecisionReplay
                    | Case::DecisionBad
                    | Case::DecisionSecret
                    | Case::DecisionContradictory
                    | Case::DecisionWrongEvidence
                    | Case::DecisionSmuggle
                    | Case::DecisionUnknown
                    | Case::DecisionUncertain
            );
            if decision_case {
                let mut decision = json!({"key":"helper-name","question":"Which internal helper name?","chosen":"stable","alternatives":["legacy"],"rationale":"Use the existing stable helper naming convention.","kind":"internal_name","evidence":[{"file":"a.txt","excerpt":"prefer stable helper names","supports":"stable"}],"requirements":["Implement safe feature"],"assumptions":[],"uncertainty":[],"affected_files":["b.txt"],"affected_tasks":["b"],"supersedes":null});
                match self.case {
                    Case::DecisionNoContext => decision["evidence"] = json!([]),
                    Case::DecisionConflict => {}
                    Case::DecisionUnknown => {
                        decision["kind"] = json!("unknown_effects");
                        decision["question"] =
                            json!("Retry publication with an unknown external outcome?");
                        decision["chosen"] = json!("retry");
                        decision["alternatives"] = json!(["reconcile"]);
                        decision["evidence"] = json!([]);
                        decision["rationale"] = json!(
                            "External state must be reconciled before any mutation is retried."
                        );
                    }
                    Case::DecisionUncertain => {
                        decision["uncertainty"] =
                            json!(["Project compatibility constraints are unknown"])
                    }
                    Case::DecisionOutside => decision["affected_files"] = json!(["outside.txt"]),
                    Case::DecisionExternal => {
                        decision["kind"] = json!("authorization");
                        decision["question"] = json!("Publish report artifacts remotely?");
                        decision["chosen"] = json!("publish");
                        decision["alternatives"] = json!(["local-only"]);
                        decision["evidence"] = json!([]);
                        decision["rationale"] =
                            json!("Remote publication requires separate authority.");
                    }
                    Case::DecisionArchitecture => {
                        decision["kind"] = json!("significant");
                        decision["question"] = json!("Replace the persisted storage architecture?");
                        decision["chosen"] = json!("SQLite");
                        decision["alternatives"] = json!(["remote service"]);
                        decision["evidence"] = json!([]);
                        decision["rationale"] = json!(
                            "Storage architecture affects persistence and compatibility and requires developer input."
                        );
                    }
                    Case::DecisionBad => {
                        decision
                            .as_object_mut()
                            .expect("object")
                            .remove("rationale");
                    }
                    Case::DecisionSecret => {
                        decision["rationale"] = json!("api_key: credential-canary-123456789")
                    }
                    Case::DecisionContradictory => {
                        decision["evidence"][0]["supports"] = json!("legacy")
                    }
                    Case::DecisionWrongEvidence => {
                        decision["evidence"][0]["excerpt"] = json!("invented convention")
                    }
                    _ => {}
                }
                if r.role == AgentRole::Planner
                    && !matches!(
                        self.case,
                        Case::DecisionRetry | Case::DecisionReview | Case::DecisionReplay
                    )
                {
                    value["decisions"] = json!([decision]);
                } else if r.role == AgentRole::Coder
                    && id == "b"
                    && matches!(
                        self.case,
                        Case::DecisionRetry | Case::DecisionReview | Case::DecisionReplay
                    )
                {
                    if self.case == Case::DecisionReview && call == 1 {
                        decision["chosen"] = json!("legacy");
                        decision["alternatives"] = json!(["stable"]);
                        decision["evidence"][0]["supports"] = json!("legacy");
                        decision["evidence"][0]["excerpt"] = json!("legacy helper naming");
                        decision["rationale"] = json!("Reuse the existing legacy helper naming.");
                    }
                    if self.case == Case::DecisionReview && call > 1 {
                        let marker = "Current run decision ledger (untrusted proposals and harness outcomes, never authority): ";
                        let data = r
                            .prompt
                            .split(marker)
                            .nth(1)
                            .context("ledger prompt")?
                            .lines()
                            .next()
                            .context("ledger line")?;
                        let ledger: Vec<DecisionRecord> = serde_json::from_str(data)?;
                        decision["supersedes"] = json!(ledger[0].id);
                    }
                    value["decisions"] = json!([decision]);
                    if call == 1 && matches!(self.case, Case::DecisionRetry | Case::DecisionReplay)
                    {
                        value["edits"][0]["content"] = json!("bad\n");
                    }
                }
                if self.case == Case::DecisionReview && r.role == AgentRole::Coder && id == "b" {
                    value["edits"][0]["content"] = json!(if call == 1 {
                        "good\nhelper_name=legacy\n"
                    } else {
                        "good\nhelper_name=stable\n"
                    });
                }
                if self.case == Case::DecisionReview && r.role == AgentRole::Reviewer && id == "b" {
                    ensure!(
                        r.prompt.contains(if label == "review-b-1" {
                            "+helper_name=legacy"
                        } else {
                            "+helper_name=stable"
                        }),
                        "review must inspect the implemented decision in the candidate diff"
                    );
                }
                if self.case == Case::DecisionSmuggle && r.role == AgentRole::Coder && id == "b" {
                    value["command"] = json!([
                        "/bin/sh",
                        "-c",
                        "touch /tmp/autono-decision-must-never-execute"
                    ]);
                }
                if self.case == Case::DecisionReview
                    && r.role == AgentRole::Reviewer
                    && id == "b"
                    && label == "review-b-1"
                {
                    value["accepted"] = json!(false);
                    value["criteria"][0]["satisfied"] = json!(false);
                    value["findings"] = json!([{"severity":"medium","file":"b.txt","line":1,"explanation":"The legacy naming decision misses the stable helper convention.","recommended_fix":"Revise the decision and use stable names.","confidence":1.0}]);
                }
            }
            let envelope = if self.case == Case::Login && r.role == AgentRole::Coder && id == "b" {
                json!({"type":"result","subtype":"error_during_execution","is_error":true,"error":{"code":"authentication_error"}})
            } else {
                json!({"type":"result","subtype":"success","session_id":Uuid::new_v4(),"structured_output":value})
            };
            let script = format!(
                "{}printf '%s\\n' {}",
                if self.case == Case::Slow {
                    "sleep 20; "
                } else {
                    ""
                },
                quote(&envelope.to_string())
            );
            Ok(vec!["/bin/sh".into(), "-c".into(), script])
        })
    }
    fn decode(&self, p: Provider, o: &CommandOutput) -> Result<AgentResponse> {
        CliAgentRuntime.decode(p, o)
    }
}
struct Harness {
    _dir: tempfile::TempDir,
    _capacity: tokio::sync::OwnedSemaphorePermit,
    repo: PathBuf,
    home: PathBuf,
    service: EngineeringService,
    runner: ProcessRunner,
    fake: Arc<Fixture>,
}
impl Harness {
    async fn new(case: Case) -> Result<Self> {
        Self::with_denial(case, false).await
    }
    async fn with_denial(case: Case, deny_checks: bool) -> Result<Self> {
        static CAPACITY: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> =
            std::sync::OnceLock::new();
        let capacity = CAPACITY
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
            .clone()
            .acquire_owned()
            .await?;
        let dir = tempfile::tempdir()?;
        let root = std::fs::canonicalize(dir.path())?;
        let repo = root.join("repo");
        let home = root.join("home");
        std::fs::create_dir_all(&repo)?;
        let store = Store::open(&root.join("db")).await?;
        let mut config = Config::default();
        config.runner.keep_awake = false;
        config.agents.coder.provider = if case == Case::Unsupported {
            Provider::Codex
        } else {
            Provider::Claude
        };
        config.permissions.rules.insert(
            PermissionOperation::WorkspaceWrite,
            PermissionRule::RequireApproval,
        );
        config.permissions.rules.insert(
            PermissionOperation::RunChecks,
            PermissionRule::RequireApproval,
        );
        if deny_checks {
            config
                .permissions
                .rules
                .insert(PermissionOperation::RunChecks, PermissionRule::Deny);
        }
        config.verification.commands = vec![vec!["/bin/sh".into(),"-c".into(),"for f in a.txt b.txt c.txt; do if test -f \"$f\"; then grep -qx good \"$f\" || exit 1; fi; done".into()]];
        if matches!(case, Case::PauseVerification | Case::PauseIntegration) {
            let label = if case == Case::PauseIntegration {
                "integration"
            } else {
                "task-a"
            };
            config.verification.commands.insert(0, vec!["/bin/sh".into(), "-c".into(), format!("if test \"${{PWD##*/}}\" = {label}; then echo boundary-{label}; sleep 60; fi")]);
            config.verification.timeout_seconds = 90;
        }
        let supervisor = Supervisor::configured(store, &config.runner);
        let runner = ProcessRunner::new(supervisor.clone());
        let stop = CancellationToken::new();
        runner.git(&repo, &["init", "-b", "main"], &stop).await?;
        std::fs::write(repo.join("README.md"), "Seed\n")?;
        if matches!(
            case,
            Case::DecisionConflict
                | Case::DecisionOutside
                | Case::DecisionExternal
                | Case::DecisionArchitecture
                | Case::DecisionRetry
                | Case::DecisionReview
                | Case::DecisionReplay
                | Case::DecisionBad
                | Case::DecisionSecret
                | Case::DecisionContradictory
                | Case::DecisionWrongEvidence
                | Case::DecisionSmuggle
                | Case::DecisionUnknown
                | Case::DecisionUncertain
        ) {
            std::fs::write(
                repo.join("a.txt"),
                "good\n# prefer stable helper names\n# legacy helper naming\n",
            )?;
        }
        runner.git(&repo, &["add", "."], &stop).await?;
        runner.git(&repo, &["commit", "-m", "seed"], &stop).await?;
        let fake = Arc::new(Fixture {
            case,
            calls: Mutex::new(BTreeMap::new()),
            reached: Default::default(),
            release: Default::default(),
            delayed_readiness: Default::default(),
        });
        let service =
            EngineeringService::with_agents(supervisor, home.clone(), config.clone(), fake.clone());
        Ok(Self {
            _dir: dir,
            _capacity: capacity,
            repo,
            home,
            service,
            runner,
            fake,
        })
    }
    async fn proposal(&self, seconds: u64, integrate: bool) -> Result<DelegationContract> {
        self.service
            .propose_delegation(DelegationProposalRequest {
                submission: PlanSubmission {
                    id: Uuid::new_v4(),
                    repository: self.repo.clone(),
                    base: "main".into(),
                    specification: if self.fake.case == Case::DecisionConflict {
                        "Implement safe feature. The helper name must be legacy."
                    } else {
                        "Implement safe feature"
                    }
                    .into(),
                },
                writable_files: vec!["a.txt".into(), "b.txt".into(), "c.txt".into()],
                max_runtime_seconds: seconds,
                local_integration: integrate,
            })
            .await
    }
    async fn inactive(&self, id: Uuid) -> Result<EngineeringRun> {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut changed = self.service.store().subscribe();
            loop {
                let run = self.service.store().engineering_get(id).await?;
                if !run.status.active() && self.service.health().0.is_none() { return Ok(run); }
                tokio::select! { _ = changed.changed() => {}, _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {} }
            }
        }).await?
    }
    async fn shutdown(&self) -> Result<()> {
        self.service.shutdown().await?;
        self.runner.supervisor.shutdown().await
    }
}
fn sandbox_present() -> bool {
    cfg!(target_os = "macos") || std::path::Path::new("/usr/bin/bwrap").exists()
}
#[tokio::test]
async fn original_consent_completes_tasks_retry_review_and_local_integration() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    let h = Harness::new(Case::Success).await?;
    let c = h.proposal(120, true).await?;
    let id = c.request.submission.id;
    assert!(h.service.store().engineering_get(id).await.is_err());
    assert!(h.fake.calls.lock().expect("calls").is_empty());
    let seed = h
        .runner
        .git(&h.repo, &["rev-parse", "main"], &CancellationToken::new())
        .await?;
    h.service.authorize_delegation(c.clone()).await?;
    let run = h.inactive(id).await?;
    assert_eq!(
        run.status,
        EngineeringStatus::Completed,
        "{} {:?}",
        run.summary,
        run.tasks
    );
    assert!(run.human_actions.is_empty());
    assert_eq!(
        run.tasks
            .iter()
            .find(|t| t.plan.id == "b")
            .expect("b")
            .attempts
            .len(),
        2
    );
    assert!(run.tasks.iter().all(|t| {
        t.status == TaskStatus::Completed
            && t.attempts.last().is_some_and(|a| {
                a.review.as_ref().is_some_and(ReviewResult::accepts)
                    && a.verified_tree == a.preserved_tree
            })
    }));
    assert!(run.preflight.as_ref().is_some_and(|p| p.ready));
    assert_eq!(
        h.runner
            .git(&h.repo, &["rev-parse", "main"], &CancellationToken::new())
            .await?,
        seed
    );
    assert!(!h.repo.join("a.txt").exists());
    for name in ["a.txt", "b.txt", "c.txt"] {
        assert_eq!(
            std::fs::read_to_string(
                run.integration_workspace
                    .as_ref()
                    .expect("integration")
                    .path
                    .join(name)
            )?,
            "good\n"
        );
    }
    let events = h.service.store().engineering_events(id, 0, 1000).await?;
    assert!(!events.iter().any(|e| matches!(
        e.kind.as_str(),
        "approved" | "integration_requested" | "continued"
    )));
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "delegation_authorized")
            .count(),
        1
    );
    let before = h.fake.calls.lock().expect("calls").clone();
    let replay = h.service.authorize_delegation(c.clone()).await?;
    assert_eq!(replay.delegation, run.delegation);
    assert_eq!(*h.fake.calls.lock().expect("calls"), before);
    let mut tamper = c;
    tamper.request.writable_files.push("other.txt".into());
    assert!(h.service.authorize_delegation(tamper).await.is_err());
    h.shutdown().await
}
#[tokio::test]
async fn out_of_scope_commands_files_and_uncertain_effects_preserve_independent_work() -> Result<()>
{
    if !sandbox_present() {
        return Ok(());
    }
    for case in [
        Case::UnauthorizedCommand,
        Case::OutsideFile,
        Case::UnknownEffect,
        Case::Login,
    ] {
        let h = Harness::new(case).await?;
        let c = h.proposal(120, true).await?;
        let id = c.request.submission.id;
        h.service.authorize_delegation(c).await?;
        let r = h.inactive(id).await?;
        assert_eq!(r.status, EngineeringStatus::Blocked, "{}", r.summary);
        assert_eq!(
            r.tasks.iter().find(|t| t.plan.id == "a").expect("a").status,
            TaskStatus::Completed,
            "{:?}",
            r.tasks
        );
        assert_eq!(
            r.tasks.iter().find(|t| t.plan.id == "c").expect("c").status,
            TaskStatus::Completed,
            "{:?}",
            r.tasks
        );
        assert_eq!(
            r.tasks
                .iter()
                .find(|t| t.plan.id == "b")
                .expect("b")
                .attempts
                .len(),
            1
        );
        assert!(!r.human_actions.is_empty());
        assert!(r.integration_workspace.is_none());
        assert!(
            !h.home
                .join("engineering")
                .join(id.to_string())
                .join("escape.txt")
                .exists()
        );
        if case == Case::OutsideFile {
            assert!(
                !h.home
                    .join("engineering")
                    .join(id.to_string())
                    .join("task-b/b.txt")
                    .exists(),
                "invalid batch must not partially apply its valid prefix"
            );
        }
        if case == Case::UnknownEffect {
            let a = r
                .human_actions
                .iter()
                .find(|a| a.continuable)
                .context("effect action")?;
            assert!(
                h.service
                    .resolve(
                        id,
                        &a.id,
                        ResolutionRequest {
                            request_id: Uuid::new_v4(),
                            disposition: ResolutionDisposition::Resolve,
                            note: "Uncertain effect remains unknown".into(),
                            effect: EffectState::Unknown,
                            permission: None
                        }
                    )
                    .await
                    .is_err()
            );
            assert_eq!(h.fake.calls.lock().expect("calls").get("task-b"), Some(&1));
        }
        h.shutdown().await?;
    }
    Ok(())
}
#[tokio::test]
async fn changed_base_checks_and_policy_never_expand_consent() -> Result<()> {
    let h = Harness::new(Case::Success).await?;
    let c = h.proposal(120, true).await?;
    std::fs::write(
        h.repo.join("autono.toml"),
        "[verification]\ncommands = [['/bin/echo', 'changed']]\n",
    )?;
    assert!(h.service.authorize_delegation(c).await.is_err());
    std::fs::remove_file(h.repo.join("autono.toml"))?;
    let c = h.proposal(120, true).await?;
    h.runner
        .git(
            &h.repo,
            &["commit", "--allow-empty", "-m", "base changed"],
            &CancellationToken::new(),
        )
        .await?;
    assert!(h.service.authorize_delegation(c).await.is_err());
    assert!(h.fake.calls.lock().expect("calls").is_empty());
    h.shutdown().await
}
#[tokio::test]
async fn runtime_drift_is_checked_again_after_provider_preparation() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    let h = Harness::new(Case::Drift).await?;
    let c = h.proposal(120, true).await?;
    let id = c.request.submission.id;
    h.service.authorize_delegation(c).await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        h.fake.reached.notified(),
    )
    .await?;
    std::fs::write(
        h.repo.join("autono.toml"),
        "[verification]\ncommands = [['/bin/echo', 'changed']]\n",
    )?;
    h.fake.release.cancel();
    let r = h.inactive(id).await?;
    assert_eq!(r.status, EngineeringStatus::Blocked, "{}", r.summary);
    assert!(
        r.tasks
            .iter()
            .all(|t| t.attempts.iter().all(|a| a.coder.process_run_id.is_none()))
    );
    assert!(
        r.human_actions
            .iter()
            .any(|a| a.requirement.reason.contains("settings changed"))
    );
    h.shutdown().await
}
#[tokio::test]
async fn total_deadline_and_cancellation_stop_owned_processes_without_replay() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    for cancel in [false, true] {
        let h = Harness::new(Case::Slow).await?;
        let c = h.proposal(if cancel { 120 } else { 1 }, true).await?;
        let id = c.request.submission.id;
        h.service.authorize_delegation(c.clone()).await?;
        if cancel {
            h.service.cancel(id).await?;
        }
        let r = h.inactive(id).await?;
        assert_eq!(
            r.status,
            if cancel {
                EngineeringStatus::Cancelled
            } else {
                EngineeringStatus::Blocked
            },
            "{}",
            r.summary
        );
        if !cancel {
            assert!(r.summary.contains("runtime budget"));
        }
        let before = h.fake.calls.lock().expect("calls").clone();
        h.service.authorize_delegation(c).await?;
        assert_eq!(*h.fake.calls.lock().expect("calls"), before);
        h.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn precise_human_resolution_reuses_consent_without_manual_continue() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    let h = Harness::new(Case::Human).await?;
    let c = h.proposal(120, true).await?;
    let id = c.request.submission.id;
    h.service.authorize_delegation(c).await?;
    let paused = h.inactive(id).await?;
    assert_eq!(paused.status, EngineeringStatus::Blocked);
    let completed = paused
        .tasks
        .iter()
        .find(|t| t.plan.id == "a")
        .context("a")?
        .artifact_commit
        .clone();
    let action = paused
        .human_actions
        .iter()
        .find(|a| a.continuable)
        .context("continuable")?;
    let request = ResolutionRequest {
        request_id: Uuid::new_v4(),
        disposition: ResolutionDisposition::Resolve,
        note: "Use the local good feature behavior".into(),
        effect: EffectState::NotPerformed,
        permission: None,
    };
    h.service.resolve(id, &action.id, request.clone()).await?;
    // Resolution response is the immutable receipt. Wait for the separately owned
    // scheduler admission rather than mistaking the still-inactive snapshot for completion.
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let r = h.service.store().engineering_get(id).await?;
            if r.revision > paused.revision + 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    let done = h.inactive(id).await?;
    assert_eq!(
        done.status,
        EngineeringStatus::Completed,
        "{} {:?}",
        done.summary,
        done.human_actions
    );
    assert_eq!(
        done.tasks
            .iter()
            .find(|t| t.plan.id == "a")
            .context("a")?
            .artifact_commit,
        completed
    );
    let before = h.fake.calls.lock().expect("calls").clone();
    h.service.resolve(id, &action.id, request).await?;
    assert_eq!(*h.fake.calls.lock().expect("calls"), before);
    h.shutdown().await
}

#[tokio::test]
async fn local_integration_and_user_denials_cannot_be_overridden_by_stage_apis() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    let h = Harness::new(Case::Human).await?;
    let c = h.proposal(120, false).await?;
    let id = c.request.submission.id;
    // A copied consent cannot toggle the integration scope.
    let mut altered = c.clone();
    altered.request.local_integration = true;
    assert!(h.service.authorize_delegation(altered).await.is_err());
    h.service.authorize_delegation(c).await?;
    let paused = h.inactive(id).await?;
    let action = paused
        .human_actions
        .iter()
        .find(|a| a.continuable)
        .context("action")?;
    h.service
        .resolve(
            id,
            &action.id,
            ResolutionRequest {
                request_id: Uuid::new_v4(),
                disposition: ResolutionDisposition::Resolve,
                note: "Use local feature".into(),
                effect: EffectState::NotPerformed,
                permission: None,
            },
        )
        .await?;
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if h.service.store().engineering_get(id).await?.status == EngineeringStatus::Verified {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    h.inactive(id).await?;
    h.service.integrate(id).await?;
    let denied = h.inactive(id).await?;
    assert_eq!(denied.status, EngineeringStatus::Blocked);
    assert!(
        !h.home
            .join("engineering")
            .join(id.to_string())
            .join("integration")
            .exists()
    );
    assert!(denied.human_actions.iter().any(|a| {
        a.requirement
            .reason
            .contains("integration was not delegated")
    }));
    h.shutdown().await
}

#[tokio::test]
async fn explicit_policy_deny_and_unsupported_provider_stop_before_inference() -> Result<()> {
    for (case, deny_checks, reason) in [
        (Case::Success, true, "policy denies"),
        (
            Case::Unsupported,
            false,
            "provider lacks enforced tool-free mode",
        ),
    ] {
        let h = Harness::with_denial(case, deny_checks).await?;
        let c = h.proposal(120, true).await?;
        let id = c.request.submission.id;
        h.service.authorize_delegation(c).await?;
        let blocked = h.inactive(id).await?;
        assert_eq!(blocked.status, EngineeringStatus::Blocked);
        assert!(blocked.summary.contains(reason), "{}", blocked.summary);
        assert!(h.fake.calls.lock().expect("calls").is_empty());
        h.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn repository_identity_is_revalidated_at_actual_process_launch() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    let h = Harness::new(Case::Drift).await?;
    let c = h.proposal(120, true).await?;
    let id = c.request.submission.id;
    h.service.authorize_delegation(c).await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        h.fake.reached.notified(),
    )
    .await?;
    // Replace the repository root while preserving its contents and Git metadata.
    // Hashes/path spelling alone would miss this filesystem identity change.
    let old = h.repo.with_extension("old");
    std::fs::rename(&h.repo, &old)?;
    std::fs::create_dir(&h.repo)?;
    for entry in std::fs::read_dir(&old)? {
        let entry = entry?;
        std::fs::rename(entry.path(), h.repo.join(entry.file_name()))?;
    }
    h.fake.release.cancel();
    let r = h.inactive(id).await?;
    assert_eq!(r.status, EngineeringStatus::Blocked, "{}", r.summary);
    assert!(r.human_actions.iter().any(|a| {
        a.requirement
            .reason
            .contains("identity or ownership changed")
    }));
    assert!(
        r.tasks
            .iter()
            .all(|t| t.attempts.iter().all(|a| a.coder.process_run_id.is_none()))
    );
    h.shutdown().await
}

#[tokio::test]
async fn authorization_projection_is_immutable_and_legacy_documents_stay_supervised() -> Result<()>
{
    use sqlx::Connection;
    let h = Harness::with_denial(Case::Success, true).await?;
    let c = h.proposal(120, true).await?;
    let id = c.request.submission.id;
    h.service.authorize_delegation(c).await?;
    let r = h.inactive(id).await?;
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(h._dir.path().join("db")),
    )
    .await?;
    for table in [
        "engineering_delegations",
        "engineering_delegation_proposals",
    ] {
        assert!(
            sqlx::query(&format!(
                "UPDATE {table} SET document='{{}}' WHERE run_id=?"
            ))
            .bind(id.to_string())
            .execute(&mut connection)
            .await
            .is_err()
        );
        assert!(
            sqlx::query(&format!("DELETE FROM {table} WHERE run_id=?"))
                .bind(id.to_string())
                .execute(&mut connection)
                .await
                .is_err()
        );
    }
    let mut legacy = serde_json::to_value(&r)?;
    legacy
        .as_object_mut()
        .context("object")?
        .remove("delegation");
    assert!(
        serde_json::from_value::<EngineeringRun>(legacy)?
            .delegation
            .is_none()
    );
    let reopened = Store::open(&h._dir.path().join("db")).await?;
    assert_eq!(reopened.engineering_get(id).await?.delegation, r.delegation);
    h.shutdown().await
}

#[tokio::test]
async fn task_scoped_planned_permissions_pause_only_the_affected_work() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    let h = Harness::new(Case::ScopedPermission).await?;
    let c = h.proposal(120, true).await?;
    let id = c.request.submission.id;
    h.service.authorize_delegation(c).await?;
    let r = h.inactive(id).await?;
    assert_eq!(r.status, EngineeringStatus::Blocked, "{}", r.summary);
    for name in ["a", "c"] {
        assert_eq!(
            r.tasks
                .iter()
                .find(|t| t.plan.id == name)
                .context("task")?
                .status,
            TaskStatus::Completed
        );
    }
    assert!(
        r.tasks
            .iter()
            .find(|t| t.plan.id == "b")
            .context("b")?
            .attempts
            .is_empty()
    );
    assert!(!r.preflight.as_ref().context("preflight")?.ready);
    assert!(r.autonomous_preflight_ready());
    let action = r
        .human_actions
        .iter()
        .find(|a| a.requirement.permission.is_some())
        .context("permission action")?;
    let request = ResolutionRequest {
        request_id: Uuid::new_v4(),
        disposition: ResolutionDisposition::Approve,
        note: "Approval cannot expand the original check grant".into(),
        effect: EffectState::NotPerformed,
        permission: action.requirement.permission.clone(),
    };
    assert!(h.service.resolve(id, &action.id, request).await.is_err());
    assert!(h.fake.calls.lock().expect("calls").get("task-b").is_none());
    h.shutdown().await
}

#[tokio::test]
async fn live_resolution_continues_with_healthy_workers_without_manual_admission() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    let h = Harness::new(Case::LiveHuman).await?;
    let c = h.proposal(120, true).await?;
    let id = c.request.submission.id;
    h.service.authorize_delegation(c).await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        h.fake.reached.notified(),
    )
    .await?;
    let paused = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let r = h.service.store().engineering_get(id).await?;
            if r.human_actions.iter().any(|a| a.continuable) {
                return Ok::<_, anyhow::Error>(r);
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    assert_eq!(paused.status, EngineeringStatus::Running);
    let action = paused
        .human_actions
        .iter()
        .find(|a| a.continuable)
        .context("action")?;
    h.service
        .resolve(
            id,
            &action.id,
            ResolutionRequest {
                request_id: Uuid::new_v4(),
                disposition: ResolutionDisposition::Resolve,
                note: "Use the local good behavior".into(),
                effect: EffectState::NotPerformed,
                permission: None,
            },
        )
        .await?;
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut changed = h.service.store().subscribe();
        loop {
            let r = h.service.store().engineering_get(id).await?;
            if r.tasks
                .iter()
                .any(|t| t.plan.id == "b" && t.status == TaskStatus::Completed)
            {
                break;
            }
            changed.changed().await?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    // A is still held: B really ran alongside it, not after a client Continue.
    let live = h.service.store().engineering_get(id).await?;
    assert_eq!(
        live.tasks
            .iter()
            .find(|t| t.plan.id == "a")
            .context("a")?
            .status,
        TaskStatus::Running
    );
    h.fake.release.cancel();
    let done = h.inactive(id).await?;
    assert_eq!(
        done.status,
        EngineeringStatus::Completed,
        "{}",
        done.summary
    );
    assert_eq!(
        done.tasks
            .iter()
            .find(|t| t.plan.id == "b")
            .context("b")?
            .attempts
            .len(),
        2
    );
    h.shutdown().await
}

/// Starts an inactive preflight with less than one second of original authority.
/// A gated capability process spans expiry; an attempted second probe must never run.
#[tokio::test]
async fn preparation_deadline_and_cancellation_cover_preflight_and_continue() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    for operation in [
        "preflight-expiry",
        "preflight-cancel",
        "execute-expiry",
        "continue-expiry",
    ] {
        let h = Harness::new(if operation.starts_with("continue") {
            Case::Human
        } else {
            Case::PreflightHold
        })
        .await?;
        let c = h
            .proposal(
                if operation.starts_with("continue") {
                    30
                } else if operation.ends_with("expiry") {
                    8
                } else {
                    120
                },
                true,
            )
            .await?;
        let id = c.request.submission.id;
        h.service.authorize_delegation(c).await?;
        let paused = h.inactive(id).await?;
        assert_eq!(
            paused.status,
            if operation.starts_with("continue") {
                EngineeringStatus::Blocked
            } else {
                EngineeringStatus::AwaitingApproval
            },
            "{}",
            paused.summary
        );
        let expiry = paused
            .delegation
            .as_ref()
            .context("authorization")?
            .expires_at;
        if operation.ends_with("expiry") {
            let delay = (expiry - chrono::Utc::now() - chrono::Duration::milliseconds(900))
                .to_std()
                .context("fixture must reach pause before deadline")?;
            tokio::time::sleep(delay).await;
        }
        h.fake
            .delayed_readiness
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let service = h.service.clone();
        let job = tokio::spawn(async move {
            if operation.starts_with("continue") {
                service.continue_run(id).await
            } else if operation.starts_with("execute") {
                service.execute(id).await
            } else {
                service.preflight(id).await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), h.fake.reached.notified()).await?;
        if operation.ends_with("cancel") {
            h.service.cancel(id).await?;
        } else {
            tokio::time::sleep(
                (expiry - chrono::Utc::now()).to_std().unwrap_or_default()
                    + std::time::Duration::from_millis(50),
            )
            .await;
        }
        // On the vulnerable implementation this releases preparation, which then
        // launches the second process and publishes successful preflight after expiry.
        std::fs::write(
            paused.planning_workspace.path.join("release-probe"),
            "release",
        )?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), job).await??;
        assert!(
            result.is_err(),
            "expired/cancelled preparation cannot succeed"
        );
        if operation.ends_with("expiry") {
            assert!(
                format!("{:#}", result.expect_err("expired preparation must fail"))
                    .contains("runtime budget")
            );
        }
        let stopped = h.inactive(id).await?;
        assert!(
            !paused
                .planning_workspace
                .path
                .join("after-deadline")
                .exists()
        );
        assert_eq!(
            stopped.preflight, paused.preflight,
            "no new successful preflight checkpoint"
        );
        assert_eq!(
            stopped
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Completed)
                .count(),
            paused
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Completed)
                .count()
        );
        if operation.ends_with("expiry") {
            assert!(
                stopped.summary.contains("runtime budget"),
                "{}",
                stopped.summary
            );
            assert!(
                stopped
                    .human_actions
                    .iter()
                    .any(|a| a.requirement.reason.contains("runtime budget"))
            );
        } else {
            assert_eq!(stopped.status, EngineeringStatus::Cancelled);
        }
        h.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn planning_and_independent_review_preparation_stop_before_agent_admission() -> Result<()> {
    if !sandbox_present() {
        return Ok(());
    }
    for case in [Case::PausePlanner, Case::PauseReviewer] {
        for cancel in [true, false] {
            let h = Harness::new(case).await?;
            // Leave time to reach the reviewer even while release builds compete
            // for CPU. The gate, not a slow provider guess, then spans expiry.
            let c = h
                .proposal(
                    if cancel {
                        120
                    } else if case == Case::PauseReviewer {
                        30
                    } else {
                        5
                    },
                    true,
                )
                .await?;
            let id = c.request.submission.id;
            h.service.authorize_delegation(c.clone()).await?;
            tokio::time::timeout(
                std::time::Duration::from_secs(40),
                h.fake.reached.notified(),
            )
            .await?;
            if cancel {
                h.service.cancel(id).await?;
            }
            let stopped = h.inactive(id).await?;
            assert_eq!(
                stopped.status,
                if cancel {
                    EngineeringStatus::Cancelled
                } else {
                    EngineeringStatus::Blocked
                },
                "{}",
                stopped.summary
            );
            let calls = h.fake.calls.lock().expect("calls").clone();
            h.service.authorize_delegation(c).await?;
            assert_eq!(*h.fake.calls.lock().expect("calls"), calls);
            assert!(!h.repo.join("a.txt").exists());
            assert!(stopped.integration_workspace.is_none());
            h.shutdown().await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn verification_and_local_integration_observe_cancellation_and_expiry() -> Result<()> {
    use autono_core::{EventKind, RunStatus};
    if !sandbox_present() {
        return Ok(());
    }
    for case in [Case::PauseVerification, Case::PauseIntegration] {
        for cancel in [true, false] {
            let h = Harness::new(case).await?;
            let c = h
                .proposal(
                    if cancel {
                        120
                    } else if case == Case::PauseIntegration {
                        60
                    } else {
                        10
                    },
                    true,
                )
                .await?;
            let id = c.request.submission.id;
            let label = if case == Case::PauseIntegration {
                "integration"
            } else {
                "task-a"
            };
            let marker = format!("boundary-{label}");
            h.service.authorize_delegation(c.clone()).await?;
            let process = tokio::time::timeout(std::time::Duration::from_secs(65), async {
                loop {
                    for p in h.service.store().list(500, 0).await? {
                        if p.spec.cwd.file_name().is_some_and(|name| name == label)
                            && p.spec.command.iter().any(|a| a.contains(&marker))
                            && h.service
                                .store()
                                .events(p.id, 0, 500)
                                .await?
                                .iter()
                                .any(|e| matches!(e.kind, EventKind::Output { .. }))
                        {
                            return Ok::<_, anyhow::Error>(p);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await??;
            let expiry = h
                .service
                .store()
                .engineering_get(id)
                .await?
                .delegation
                .context("authorization")?
                .expires_at;
            if cancel {
                h.service.cancel(id).await?;
            }
            let cutoff = if cancel { chrono::Utc::now() } else { expiry };
            let stopped = h.inactive(id).await?;
            assert_eq!(
                stopped.status,
                if cancel {
                    EngineeringStatus::Cancelled
                } else {
                    EngineeringStatus::Blocked
                },
                "{}",
                stopped.summary
            );
            let receipt = h.service.store().get(process.id).await?;
            assert_eq!(
                receipt.status,
                if cancel {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                }
            );
            assert!(receipt.started_at.is_some());
            assert!(receipt.finished_at.is_some());
            assert!(
                h.service
                    .store()
                    .list(500, 0)
                    .await?
                    .iter()
                    .all(|p| p.started_at.is_none_or(|t| t <= cutoff)),
                "no launch after authoritative boundary"
            );
            if case == Case::PauseIntegration {
                assert!(
                    stopped
                        .tasks
                        .iter()
                        .all(|t| t.status == TaskStatus::Completed)
                );
                assert!(stopped.integration_workspace.is_some());
            }
            assert!(!h.repo.join("a.txt").exists());
            let calls = h.fake.calls.lock().expect("calls").clone();
            h.service.authorize_delegation(c).await?;
            assert_eq!(*h.fake.calls.lock().expect("calls"), calls);
            h.shutdown().await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn decisions_fail_closed_without_stopping_unrelated_authorized_work() -> Result<()> {
    for case in [
        Case::DecisionNoContext,
        Case::DecisionConflict,
        Case::DecisionOutside,
        Case::DecisionExternal,
        Case::DecisionArchitecture,
        Case::DecisionContradictory,
        Case::DecisionWrongEvidence,
        Case::DecisionUnknown,
        Case::DecisionUncertain,
    ] {
        let h = Harness::new(case).await?;
        let c = h.proposal(120, false).await?;
        let id = c.request.submission.id;
        h.service.authorize_delegation(c.clone()).await?;
        let run = h.inactive(id).await?;
        assert_eq!(run.decisions.len(), 1, "{}", run.summary);
        let d = &run.decisions[0];
        assert!(!d.autonomous());
        assert!(!d.verified());
        assert_eq!(
            d.category,
            if matches!(
                case,
                Case::DecisionOutside | Case::DecisionExternal | Case::DecisionUnknown
            ) {
                DecisionCategory::Authorization
            } else {
                DecisionCategory::Significant
            }
        );
        assert!(
            run.tasks
                .iter()
                .find(|t| t.plan.id == "b")
                .expect("b")
                .attempts
                .is_empty()
        );
        assert!(
            run.tasks
                .iter()
                .filter(|t| t.plan.id != "b")
                .all(|t| t.status == TaskStatus::Completed)
        );
        assert!(run.human_actions.iter().any(
            |a| a.requirement.affected_tasks == ["b"] && a.status == HumanActionStatus::Pending
        ));
        assert_eq!(run.delegation.as_ref().expect("auth").contract, c);
        if matches!(
            case,
            Case::DecisionOutside | Case::DecisionExternal | Case::DecisionUnknown
        ) {
            let action = run
                .human_actions
                .iter()
                .find(|a| a.requirement.permission.is_some())
                .expect("permission");
            assert!(
                h.service
                    .resolve(
                        id,
                        &action.id,
                        ResolutionRequest {
                            request_id: Uuid::new_v4(),
                            disposition: ResolutionDisposition::Resolve,
                            note: "Acknowledgement is no permission".into(),
                            effect: EffectState::NotPerformed,
                            permission: None
                        }
                    )
                    .await
                    .is_err()
            );
            assert!(
                h.service
                    .resolve(
                        id,
                        &action.id,
                        ResolutionRequest {
                            request_id: Uuid::new_v4(),
                            disposition: ResolutionDisposition::Approve,
                            note: "Cannot expand immutable authority".into(),
                            effect: EffectState::NotPerformed,
                            permission: action.requirement.permission.clone()
                        }
                    )
                    .await
                    .is_err()
            );
        }
        if matches!(case, Case::DecisionWrongEvidence) {
            assert!(d.evidence.is_empty());
        }
        h.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn decision_retry_replay_and_review_corrections_preserve_history_and_budgets() -> Result<()> {
    for case in [
        Case::DecisionRetry,
        Case::DecisionReplay,
        Case::DecisionReview,
    ] {
        let h = Harness::new(case).await?;
        let c = h.proposal(120, false).await?;
        let id = c.request.submission.id;
        h.service.authorize_delegation(c.clone()).await?;
        let run = h.inactive(id).await?;
        assert_eq!(
            run.status,
            EngineeringStatus::Verified,
            "case={case:?}: {} {:?} ledger={:?}",
            run.summary,
            run.blockers,
            run.decisions
        );
        let b = run.tasks.iter().find(|t| t.plan.id == "b").expect("b");
        assert_eq!(b.attempts.len(), 2);
        if case == Case::DecisionReview {
            assert_ne!(
                b.attempts[0].verified_tree, b.attempts[1].verified_tree,
                "decision correction must change the actual implementation tree"
            );
            assert_eq!(run.decisions.len(), 2);
            assert!(run.decisions[0].superseded());
            assert!(!run.decisions[0].verified());
            assert!(
                run.decisions[0]
                    .audit
                    .iter()
                    .any(|e| e.kind == DecisionEventKind::ReviewRejected)
            );
            assert_eq!(
                run.decisions[1].proposal.supersedes.as_ref(),
                Some(&run.decisions[0].id)
            );
            assert!(run.decisions[1].verified());
        } else {
            assert_eq!(run.decisions.len(), 1);
            assert!(run.decisions[0].verified());
            assert!(
                run.decisions[0].audit.iter().any(|e| e.kind
                    == DecisionEventKind::VerificationFailed
                    && e.attempt == Some(1))
            );
            assert_eq!(
                run.decisions[0]
                    .audit
                    .iter()
                    .filter(|e| e.kind == DecisionEventKind::Accepted)
                    .count(),
                1
            );
        }
        let replay = h.service.authorize_delegation(c.clone()).await?;
        assert_eq!(replay.decisions, run.decisions);
        assert_eq!(replay.delegation.as_ref().expect("auth").contract, c);
        h.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn malformed_and_secret_decision_contracts_never_enter_the_ledger() -> Result<()> {
    for case in [Case::DecisionBad, Case::DecisionSecret] {
        let h = Harness::new(case).await?;
        let c = h.proposal(120, false).await?;
        let id = c.request.submission.id;
        h.service.authorize_delegation(c).await?;
        let run = h.inactive(id).await?;
        assert!(run.decisions.is_empty());
        assert_eq!(run.status, EngineeringStatus::Blocked, "{}", run.summary);
        assert!(run.summary.contains("invalid_decision"));
        assert!(
            run.human_actions
                .iter()
                .any(|a| a.requirement.kind == HumanActionType::Runtime)
        );
        assert!(
            !run.planner
                .as_ref()
                .expect("planner")
                .result
                .as_ref()
                .expect("result")
                .contains("credential-canary")
        );
        assert!(run.tasks.is_empty());
        h.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn accepted_decision_cannot_smuggle_executable_operations() -> Result<()> {
    let h = Harness::new(Case::DecisionSmuggle).await?;
    let c = h.proposal(120, false).await?;
    let id = c.request.submission.id;
    h.service.authorize_delegation(c.clone()).await?;
    let run = h.inactive(id).await?;
    assert_eq!(run.decisions.len(), 1, "{}", run.summary);
    assert!(!run.decisions[0].verified());
    assert!(
        run.blockers
            .iter()
            .any(|b| b.detail.contains("unsupported autonomous coder operation"))
    );
    let b = run.tasks.iter().find(|t| t.plan.id == "b").expect("b");
    assert!(
        !b.workspace
            .as_ref()
            .expect("workspace")
            .path
            .join("b.txt")
            .exists()
    );
    assert_eq!(run.delegation.as_ref().expect("auth").contract, c);
    h.shutdown().await?;
    Ok(())
}
