#![cfg(unix)]
use anyhow::{Result, ensure};
use autono_core::{
    Store, Supervisor,
    config::Config,
    engineering::{
        agent::{AgentFuture, AgentRequest, AgentResponse, AgentRuntime, CliAgentRuntime},
        process::{CommandOutput, ProcessRunner},
        workspace::WorkspaceLease,
        *,
    },
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn plan(ids: &[(&str, &[&str])]) -> ExecutionPlan {
    ExecutionPlan {
        requirements: vec![Requirement {
            id: "r1".into(),
            text: "Implement the requested feature with meaningful errors".into(),
        }],
        milestones: vec![Milestone {
            id: "m1".into(),
            goal: "Verified feature".into(),
            task_ids: ids.iter().map(|(s, _)| s.to_string()).collect(),
        }],
        tasks: ids
            .iter()
            .map(|(id, deps)| PlannedTask {
                id: id.to_string(),
                goal: format!("Implement {id}"),
                description: "Implement a small testable behavior".into(),
                acceptance_criteria: vec!["The feature works and has verification evidence".into()],
                dependencies: deps.iter().map(|s| s.to_string()).collect(),
                requirement_ids: vec!["r1".into()],
                verification_expectations: vec!["Run the configured checks".into()],
                role: AgentRole::Coder,
            })
            .collect(),
    }
}
#[test]
fn plans_reject_cycles_unknown_dependencies_missing_requirements_and_excessive_decomposition() {
    let valid = plan(&[("a", &[]), ("b", &["a"]), ("c", &[])]);
    assert_eq!(valid.validate().expect("valid"), ["a", "b", "c"]);
    let mut p = valid.clone();
    p.tasks[0].dependencies.push("b".into());
    assert!(p.validate().is_err());
    let mut p = valid.clone();
    p.tasks[0].dependencies.push("unknown".into());
    assert!(p.validate().is_err());
    let mut p = valid.clone();
    p.requirements.push(Requirement {
        id: "r2".into(),
        text: "Must be retained".into(),
    });
    assert!(p.validate().is_err());
    let mut p = valid.clone();
    p.tasks[1].id = "a".into();
    assert!(p.validate().is_err());
    let mut p = valid.clone();
    p.tasks[0].acceptance_criteria.clear();
    assert!(p.validate().is_err());
    let mut p = valid;
    p.tasks = vec![p.tasks[0].clone(); 33];
    assert!(p.validate().is_err());
}
#[test]
fn task_states_and_limits_are_explicit() {
    assert!(!TaskStatus::Ready.can_transition(TaskStatus::Completed));
    assert!(!TaskStatus::Completed.can_transition(TaskStatus::Running));
    assert!(TaskStatus::Running.can_transition(TaskStatus::Blocked));
    let mut c = WorkflowSettings::default();
    c.execution.max_attempts = 0;
    assert!(c.validate().is_err());
    c.execution.max_attempts = 3;
    c.verification.commands.clear();
    assert!(c.validate().is_err());
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Success,
    CoderFailure,
    VerifyFailure,
    Fix,
    Reject,
    Human,
    ReviewerMutation,
    Concurrent,
    Conflict,
    Slow,
    Timeout,
    BlockedIndependent,
}
struct FakeAgents {
    plan: ExecutionPlan,
    scenario: Scenario,
    root: PathBuf,
    coder_calls: AtomicUsize,
    review_calls: AtomicUsize,
    requests: Mutex<Vec<(AgentRole, Option<String>, String)>>,
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
impl AgentRuntime for FakeAgents {
    fn prepare<'a>(
        &'a self,
        request: &'a AgentRequest,
        _: &'a ProcessRunner,
        _: &'a CancellationToken,
    ) -> AgentFuture<'a> {
        Box::pin(async move {
            self.requests.lock().expect("requests").push((
                request.role,
                request.resume.clone(),
                request.prompt.clone(),
            ));
            let id = request
                .cwd
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("planner")
                .strip_prefix("task-")
                .unwrap_or("a");
            let mut script = String::new();
            let value: Value = match request.role {
                AgentRole::Planner => serde_json::to_value(&self.plan)?,
                AgentRole::Coder => {
                    let number = self.coder_calls.fetch_add(1, Ordering::SeqCst);
                    if self.scenario == Scenario::CoderFailure {
                        script.push_str("echo 'deterministic coder failure' >&2; exit 42;\n");
                    }
                    if matches!(self.scenario, Scenario::Slow | Scenario::Timeout) {
                        script.push_str(&format!(
                            "touch {}; sleep 30; touch {};\n",
                            quote(self.root.join("started").to_str().expect("path")),
                            quote(self.root.join("forbidden").to_str().expect("path"))
                        ));
                    }
                    if self.scenario == Scenario::Concurrent {
                        script.push_str(&format!("touch {}; i=0; while [ ! -f {} ] || [ ! -f {} ]; do i=$((i+1)); [ $i -lt 500 ] || exit 19; sleep 0.01; done;\n",quote(self.root.join(format!("started-{id}")).to_str().expect("path")),quote(self.root.join("started-a").to_str().expect("path")),quote(self.root.join("started-b").to_str().expect("path"))));
                    }
                    if self.scenario == Scenario::Conflict {
                        script.push_str(&format!("printf '%s' {} > shared.txt;\n", quote(id)));
                    } else {
                        let text = if self.scenario == Scenario::VerifyFailure
                            || (self.scenario == Scenario::Fix && number == 0)
                        {
                            "broken"
                        } else {
                            "fixed"
                        };
                        script.push_str(&format!(
                            "printf '%s' {} > {};\n",
                            quote(text),
                            quote(&format!("feature-{id}.txt"))
                        ));
                    }
                    // Dependencies must be present in this task's isolated base.
                    for dependency in &self
                        .plan
                        .tasks
                        .iter()
                        .find(|t| t.id == id)
                        .expect("task")
                        .dependencies
                    {
                        script.push_str(&format!(
                            "test -f {} || exit 15;\n",
                            quote(&format!("feature-{dependency}.txt"))
                        ));
                    }
                    let blocker = if self.scenario == Scenario::Human
                        || (self.scenario == Scenario::BlockedIndependent && id == "a")
                    {
                        "Manual action is required"
                    } else {
                        ""
                    };
                    json!({"summary":"CODER_PRIVATE_REASONING_MUST_NOT_REACH_REVIEWER","blocker":blocker})
                }
                AgentRole::Reviewer => {
                    let number = self.review_calls.fetch_add(1, Ordering::SeqCst);
                    if self.scenario == Scenario::ReviewerMutation {
                        script.push_str("echo unauthorized >> feature-a.txt;\n");
                    }
                    let reject = self.scenario == Scenario::Reject
                        || (self.scenario == Scenario::Fix && number == 0);
                    let task = self.plan.tasks.iter().find(|t| t.id == id).expect("task");
                    json!({"accepted":!reject,"blocker":"","criteria":task.acceptance_criteria.iter().map(|c|json!({"criterion":c,"satisfied":!reject,"evidence":"Inspected the implementation and deterministic check results"})).collect::<Vec<_>>(),"findings":if reject{vec![json!({"severity":"high","file":format!("feature-{id}.txt"),"line":1,"explanation":"The implementation still returns broken","recommended_fix":"Return fixed and preserve error handling","confidence":0.99})]}else{vec![]}})
                }
            };
            let session = Uuid::new_v4();
            let events = match request.config.provider {
                Provider::Codex => format!(
                    "{}\n{}\n{}\n",
                    json!({"type":"thread.started","thread_id":session}),
                    json!({"type":"item.completed","item":{"type":"agent_message","text":serde_json::to_string(&value)?}}),
                    json!({"type":"turn.completed"})
                ),
                Provider::Claude => format!(
                    "{}\n",
                    json!({"type":"result","subtype":"success","is_error":false,"session_id":session,"structured_output":value,"permission_denials":[]})
                ),
            };
            script.push_str(&format!("printf '%s' {}", quote(&events)));
            Ok(vec!["sh".into(), "-c".into(), script])
        })
    }
    fn decode(&self, provider: Provider, output: &CommandOutput) -> Result<AgentResponse> {
        CliAgentRuntime.decode(provider, output)
    }
}
struct Harness {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    service: EngineeringService,
    supervisor: Supervisor,
    agents: Arc<FakeAgents>,
}
impl Harness {
    async fn new(scenario: Scenario, plan: ExecutionPlan, check: &str) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let root = tokio::fs::canonicalize(dir.path()).await?;
        let repo = root.join("repo");
        tokio::fs::create_dir(&repo).await?;
        let store = Store::open(&root.join("runtime.sqlite3")).await?;
        let supervisor = Supervisor::new(store, false);
        let runner = ProcessRunner {
            supervisor: supervisor.clone(),
        };
        let stop = CancellationToken::new();
        runner.git(&repo, &["init", "-b", "main"], &stop).await?;
        tokio::fs::write(repo.join("README.md"), "# Disposable test repository\n").await?;
        tokio::fs::write(repo.join("shared.txt"), "base").await?;
        tokio::fs::write(repo.join("check.sh"), check).await?;
        runner.git(&repo, &["add", "."], &stop).await?;
        runner
            .git(&repo, &["commit", "-m", "initial"], &stop)
            .await?;
        tokio::fs::write(repo.join("developer-work.txt"), "do not modify").await?;
        let mut config = Config::default();
        config.runner.keep_awake = false;
        config.verification.commands = vec![vec!["sh".into(), "check.sh".into()]];
        config.verification.timeout_seconds = 10;
        for agent in [
            &mut config.agents.planner,
            &mut config.agents.coder,
            &mut config.agents.reviewer,
        ] {
            agent.timeout_seconds = 10;
        }
        if scenario == Scenario::Timeout {
            config.agents.coder.timeout_seconds = 1;
        }
        let agents = Arc::new(FakeAgents {
            plan,
            scenario,
            root: root.clone(),
            coder_calls: AtomicUsize::new(0),
            review_calls: AtomicUsize::new(0),
            requests: Mutex::new(vec![]),
        });
        let service = EngineeringService::with_agents(
            supervisor.clone(),
            root.join("home"),
            config,
            agents.clone(),
        );
        Ok(Self {
            _dir: dir,
            root,
            repo,
            service,
            supervisor,
            agents,
        })
    }
    async fn planned(&self) -> Result<Uuid> {
        let run=self.service.plan(PlanSubmission{id:Uuid::new_v4(),repository:self.repo.clone(),base:"HEAD".into(),specification:"Implement a configuration validation CLI feature with meaningful errors and tests".into()}).await?;
        let run = self.wait(run.id).await?;
        ensure!(
            run.status == EngineeringStatus::AwaitingApproval,
            "planning failed: {}",
            run.summary
        );
        Ok(run.id)
    }
    async fn wait(&self, id: Uuid) -> Result<EngineeringRun> {
        tokio::time::timeout(std::time::Duration::from_secs(30),async{
            let mut changed=self.service.store().subscribe();
            loop{let run=self.service.store().engineering_get(id).await?;if !run.status.active()&&self.service.health().0.is_none(){return Ok(run);}tokio::select!{_ = changed.changed()=>{},_ = tokio::time::sleep(std::time::Duration::from_millis(10))=>{}}}
        }).await?
    }
    async fn execute(&self) -> Result<EngineeringRun> {
        let id = self.planned().await?;
        self.service.execute(id).await?;
        self.wait(id).await
    }
    async fn finish(self) -> Result<()> {
        self.service.shutdown().await?;
        self.supervisor.shutdown().await
    }
}

#[tokio::test]
async fn isolated_coder_verify_review_dependencies_and_controlled_integration() -> Result<()> {
    let h = Harness::new(
        Scenario::Success,
        plan(&[("a", &[]), ("b", &["a"])]),
        "test -f feature-a.txt\n",
    )
    .await?;
    let run = h.execute().await?;
    assert_eq!(run.status, EngineeringStatus::Verified, "{}", run.summary);
    assert!(run.tasks.iter().all(|t| t.status == TaskStatus::Completed));
    assert!(!h.repo.join("feature-a.txt").exists());
    assert_eq!(
        tokio::fs::read_to_string(h.repo.join("developer-work.txt")).await?,
        "do not modify"
    );
    let workspace = run.tasks[0].workspace.as_ref().expect("workspace");
    let lease = WorkspaceLease::acquire(workspace)?;
    assert!(WorkspaceLease::acquire(workspace).is_err());
    drop(lease);
    let requests = h.agents.requests.lock().expect("requests").clone();
    assert!(
        requests
            .iter()
            .filter(|r| r.0 == AgentRole::Reviewer)
            .all(|r| r.1.is_none() && !r.2.contains("CODER_PRIVATE_REASONING"))
    );
    assert!(h.service.execute(run.id).await.is_err());
    h.service.integrate(run.id).await?;
    let run = h.wait(run.id).await?;
    assert_eq!(run.status, EngineeringStatus::Completed, "{}", run.summary);
    assert!(
        run.integration_workspace
            .expect("integration")
            .head_commit
            .is_some()
    );
    let events = h.service.store().engineering_events(run.id, 0, 500).await?;
    assert!(events.iter().any(|e| e.kind == "approved"));
    assert!(events.windows(2).all(|e| e[0].sequence < e[1].sequence));
    h.finish().await
}
#[tokio::test]
async fn failures_and_review_limits_never_complete_on_agent_exit_alone() -> Result<()> {
    for (scenario, check, attempts, reviews) in [
        (Scenario::CoderFailure, "true", 3, 0),
        (Scenario::VerifyFailure, "grep -q fixed feature-a.txt", 3, 0),
        (Scenario::Reject, "true", 2, 2),
    ] {
        let h = Harness::new(scenario, plan(&[("a", &[])]), check).await?;
        let run = h.execute().await?;
        assert_eq!(run.status, EngineeringStatus::Failed, "{}", run.summary);
        assert_eq!(run.tasks[0].attempts.len(), attempts);
        assert_eq!(h.agents.review_calls.load(Ordering::SeqCst), reviews);
        assert!(run.tasks[0].artifact_commit.is_none());
        assert!(
            run.tasks[0]
                .workspace
                .as_ref()
                .expect("workspace")
                .path
                .exists()
        );
        h.finish().await?;
    }
    Ok(())
}
#[tokio::test]
async fn independent_findings_drive_a_bounded_coder_resume_and_reverification() -> Result<()> {
    let h = Harness::new(Scenario::Fix, plan(&[("a", &[])]), "test -f feature-a.txt").await?;
    let run = h.execute().await?;
    assert_eq!(run.status, EngineeringStatus::Verified, "{}", run.summary);
    assert_eq!(run.tasks[0].attempts.len(), 2);
    assert_eq!(h.agents.review_calls.load(Ordering::SeqCst), 2);
    let attempts = &run.tasks[0].attempts;
    assert!(attempts[0].review.as_ref().expect("review").findings[0].blocking());
    assert!(attempts.iter().all(|a| a.verification.len() == 1));
    assert_eq!(
        attempts[1].coder.resumed_from,
        attempts[0].coder.provider_session_id
    );
    assert_eq!(
        tokio::fs::read_to_string(
            run.tasks[0]
                .workspace
                .as_ref()
                .expect("workspace")
                .path
                .join("feature-a.txt")
        )
        .await?,
        "fixed"
    );
    h.finish().await
}
#[tokio::test]
async fn manual_blockers_and_reviewer_mutations_are_explicit_and_retained() -> Result<()> {
    for scenario in [Scenario::Human, Scenario::ReviewerMutation] {
        let h = Harness::new(scenario, plan(&[("a", &[])]), "true").await?;
        let run = h.execute().await?;
        assert_eq!(run.status, EngineeringStatus::Blocked, "{}", run.summary);
        assert_eq!(run.tasks[0].attempts.len(), 1);
        assert!(!run.blockers.is_empty());
        assert!(run.tasks[0].artifact_commit.is_none());
        h.finish().await?;
    }
    Ok(())
}
#[tokio::test]
async fn independent_tasks_run_concurrently_without_waiting_on_a_blocked_branch() -> Result<()> {
    let h = Harness::new(
        Scenario::Concurrent,
        plan(&[("a", &[]), ("b", &[])]),
        "true",
    )
    .await?;
    let run = h.execute().await?;
    assert_eq!(run.status, EngineeringStatus::Verified, "{}", run.summary);
    assert!(h.root.join("started-a").exists() && h.root.join("started-b").exists());
    h.finish().await?;
    let h = Harness::new(
        Scenario::BlockedIndependent,
        plan(&[("a", &[]), ("b", &["a"]), ("c", &["b"]), ("d", &[])]),
        "true",
    )
    .await?;
    let run = h.execute().await?;
    assert_eq!(run.status, EngineeringStatus::Blocked);
    assert_eq!(run.tasks[3].status, TaskStatus::Completed);
    assert_eq!(run.tasks[2].status, TaskStatus::Blocked);
    assert!(run.tasks[1].attempts.is_empty());
    h.finish().await
}
#[tokio::test]
async fn cancellation_stops_the_process_and_preserves_durable_task_state() -> Result<()> {
    let h = Harness::new(Scenario::Slow, plan(&[("a", &[])]), "true").await?;
    let id = h.planned().await?;
    h.service.execute(id).await?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !h.root.join("started").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    h.service.cancel(id).await?;
    let run = h.wait(id).await?;
    assert_eq!(run.status, EngineeringStatus::Cancelled, "{}", run.summary);
    assert_eq!(run.tasks[0].status, TaskStatus::Cancelled);
    assert!(!h.root.join("forbidden").exists());
    h.finish().await
}
#[tokio::test]
async fn integration_conflicts_and_combined_test_failures_are_preserved() -> Result<()> {
    for (scenario, check, expected) in [
        (Scenario::Conflict, "true", EngineeringStatus::Blocked),
        (
            Scenario::Success,
            "test ! -f feature-a.txt || test ! -f feature-b.txt",
            EngineeringStatus::Failed,
        ),
    ] {
        let h = Harness::new(scenario, plan(&[("a", &[]), ("b", &[])]), check).await?;
        let run = h.execute().await?;
        assert_eq!(run.status, EngineeringStatus::Verified, "{}", run.summary);
        h.service.integrate(run.id).await?;
        let run = h.wait(run.id).await?;
        assert_eq!(run.status, expected, "{}", run.summary);
        assert!(
            run.integration_workspace
                .expect("integration")
                .path
                .exists()
        );
        h.finish().await?;
    }
    Ok(())
}
#[tokio::test]
async fn verification_mutations_are_not_accepted_as_evidence() -> Result<()> {
    let h = Harness::new(
        Scenario::Success,
        plan(&[("a", &[])]),
        "echo change >> shared.txt",
    )
    .await?;
    let run = h.execute().await?;
    assert_eq!(run.status, EngineeringStatus::Blocked);
    assert_eq!(h.agents.review_calls.load(Ordering::SeqCst), 0);
    h.finish().await
}

#[tokio::test]
async fn agent_timeouts_are_bounded_and_do_not_start_review() -> Result<()> {
    let h = Harness::new(Scenario::Timeout, plan(&[("a", &[])]), "true").await?;
    let run = h.execute().await?;
    assert_eq!(run.status, EngineeringStatus::Failed, "{}", run.summary);
    assert_eq!(run.tasks[0].attempts.len(), 3);
    assert_eq!(h.agents.review_calls.load(Ordering::SeqCst), 0);
    assert!(
        run.tasks[0]
            .attempts
            .iter()
            .all(|a| a.coder.outcome.as_deref() == Some("timeout"))
    );
    assert!(!h.root.join("forbidden").exists());
    h.finish().await
}
#[tokio::test]
async fn dropping_plan_submission_waiter_keeps_registered_work_owned() -> Result<()> {
    let h = Harness::new(Scenario::Success, plan(&[("a", &[])]), "true").await?;
    let id = Uuid::new_v4();
    let service = h.service.clone();
    let submission = PlanSubmission {
        id,
        repository: h.repo.clone(),
        base: "HEAD".into(),
        specification: "Implement the feature".into(),
    };
    let retry = submission.clone();
    let waiter = tokio::spawn(async move { service.plan(submission).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while h.service.health().0 != Some(id) {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    waiter.abort();
    let _ = waiter.await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(r) = h.service.store().engineering_get(id).await
                && !r.status.active()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let run = h.wait(id).await?;
    assert_eq!(run.status, EngineeringStatus::AwaitingApproval);
    assert_eq!(h.service.plan(retry).await?.id, id);
    assert_eq!(
        h.agents
            .requests
            .lock()
            .expect("requests")
            .iter()
            .filter(|r| r.0 == AgentRole::Planner)
            .count(),
        1
    );
    h.finish().await
}

#[tokio::test]
async fn workflow_cancellation_does_not_wait_for_sqlite_checkpoints() -> Result<()> {
    use sqlx::Connection;
    let h = Harness::new(Scenario::Slow, plan(&[("a", &[])]), "true").await?;
    let id = h.planned().await?;
    h.service.execute(id).await?;
    let process=tokio::time::timeout(std::time::Duration::from_secs(10),async{let mut changed=h.service.store().subscribe();loop{let run=h.service.store().engineering_get(id).await?;if let Some(process)=run.tasks[0].attempts.first().and_then(|a|a.coder.process_run_id) && h.root.join("started").exists(){return Ok::<_,anyhow::Error>(process);}tokio::select!{_ = changed.changed()=>{},_ = tokio::time::sleep(std::time::Duration::from_millis(10))=>{}}}}).await??;
    let finished = h
        .supervisor
        .control_finished(process)
        .expect("owned live process");
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(h.root.join("runtime.sqlite3")),
    )
    .await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut connection)
        .await?;
    h.service.cancel(id).await?;
    tokio::time::timeout(std::time::Duration::from_secs(2), finished.cancelled()).await?;
    sqlx::query("ROLLBACK").execute(&mut connection).await?;
    let run = h.wait(id).await?;
    assert_eq!(run.status, EngineeringStatus::Cancelled);
    assert!(!h.root.join("forbidden").exists());
    h.finish().await
}
