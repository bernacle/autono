use super::{
    agent::{self, AgentFailure, AgentRequest, AgentResponse, AgentRuntime},
    journal::Journal,
    process::ProcessRunner,
    workspace::{self, WorkspaceLease},
    *,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct Engine {
    pub journal: Journal,
    pub runner: ProcessRunner,
    pub agents: Arc<dyn AgentRuntime>,
    pub home: PathBuf,
    pub stop: CancellationToken,
}
#[derive(Clone)]
enum SessionTarget {
    Planner,
    Coder(String, usize),
    Reviewer(String, usize),
}
fn task_mut<'a>(run: &'a mut EngineeringRun, id: &str) -> Result<&'a mut EngineeringTask> {
    run.tasks
        .iter_mut()
        .find(|t| t.plan.id == id)
        .context("task missing")
}
fn session_mut<'a>(
    run: &'a mut EngineeringRun,
    target: &SessionTarget,
) -> Result<&'a mut Option<AgentSession>> {
    match target {
        SessionTarget::Planner => Ok(&mut run.planner),
        SessionTarget::Reviewer(id, n) => Ok(&mut task_mut(run, id)?.attempts[*n - 1].reviewer),
        SessionTarget::Coder(_, _) => anyhow::bail!("coder session is required"),
    }
}
fn set_session(
    run: &mut EngineeringRun,
    target: &SessionTarget,
    session: AgentSession,
) -> Result<()> {
    if let SessionTarget::Coder(id, n) = target {
        task_mut(run, id)?.attempts[*n - 1].coder = session;
    } else {
        *session_mut(run, target)? = Some(session);
    }
    Ok(())
}
fn new_session(config: &AgentConfig, role: AgentRole, resume: Option<String>) -> AgentSession {
    AgentSession {
        id: Uuid::new_v4(),
        role,
        provider: config.provider,
        provider_session_id: None,
        resumed_from: resume,
        process_key: Uuid::new_v4(),
        process_run_id: None,
        outcome: None,
        result: None,
    }
}
impl Engine {
    pub async fn plan(&self) -> Result<()> {
        let run = self.journal.read().await;
        let _lease = WorkspaceLease::acquire(&run.planning_workspace).map_err(workspace_blocker)?;
        workspace::create(
            &self.runner,
            &run.repository,
            &run.planning_workspace,
            &self.stop,
        )
        .await
        .map_err(workspace_blocker)?;
        let before =
            workspace::snapshot(&self.runner, &run.planning_workspace.path, &self.stop).await?;
        let prompt = format!(
            "You are Autono's planning provider. Return only a structured execution plan, not implementation. Preserve all requirements below. Use 1..32 cohesive tasks (prefer 1..5 for small features), 1..8 milestones, stable simple IDs, dependencies, nonempty acceptance criteria and verification expectations. Every requirement must be covered by a task. Every task belongs to exactly one milestone. Only role coder is allowed. Do not change files, create commits, execute code, or propose new verification commands. Developer-configured checks: {}. Read relevant architecture in the provided checkout.\nOriginal specification (source of truth):\n{}\nProject context:\n{}",
            serde_json::to_string(&run.settings.verification.commands)?,
            run.specification,
            context(&run.planning_workspace.path).await
        );
        let response = self
            .call_agent(
                SessionTarget::Planner,
                run.settings.agents.planner.clone(),
                AgentRole::Planner,
                run.planning_workspace.path.clone(),
                prompt,
                None,
            )
            .await;
        let after =
            workspace::snapshot(&self.runner, &run.planning_workspace.path, &self.stop).await?;
        ensure!(
            before == after,
            AgentFailure {
                kind: "planner_mutation".into(),
                detail:
                    "Planner changed its read-only checkout; output rejected and workspace retained"
                        .into(),
                blocked: true
            }
        );
        let plan: ExecutionPlan =
            serde_json::from_value(response?.result).context("decode structured plan")?;
        plan.validate()?;
        self.journal.change("plan_ready","Validated plan awaits developer approval",|r| {
            r.tasks = plan.tasks.iter().map(|p| EngineeringTask { plan: p.clone(), status: if p.dependencies.is_empty() {TaskStatus::Ready} else {TaskStatus::Waiting}, workspace: None, attempts: vec![], artifact_commit: None }).collect();
            r.plan = Some(plan); r.status = EngineeringStatus::AwaitingApproval;
            r.summary = "Plan validated. Inspect requirements, dependencies, and checks; execute explicitly approves this plan.".into(); Ok(())
        }).await
    }
    async fn call_agent(
        &self,
        target: SessionTarget,
        config: AgentConfig,
        role: AgentRole,
        cwd: PathBuf,
        prompt: String,
        resume: Option<String>,
    ) -> Result<AgentResponse> {
        ensure!(!self.stop.is_cancelled(), "workflow stopped");
        let prompt = format!(
            "Use the CLI's existing login only. Never inspect, print, copy, or modify CLI credential files or authentication tokens.\n\n{prompt}"
        );
        let mut session = new_session(&config, role, resume.clone());
        self.journal
            .change(
                "agent_started",
                &format!(
                    "{:?} {:?}; session={} process_key={}",
                    role, config.provider, session.id, session.process_key
                ),
                |r| set_session(r, &target, session.clone()),
            )
            .await?;
        let run = self.journal.read().await;
        let artifacts = self
            .home
            .join("engineering")
            .join(run.id.to_string())
            .join("artifacts");
        tokio::fs::create_dir_all(&artifacts).await?;
        let schema_path = artifacts.join(format!("{}.schema.json", session.id));
        tokio::fs::write(
            &schema_path,
            serde_json::to_vec_pretty(&agent::schema(role))?,
        )
        .await?;
        let prompt_path = artifacts.join(format!("{}.prompt.md", session.id));
        tokio::fs::write(&prompt_path, &prompt).await?;
        // Keep individual argv values below Linux/macOS limits for large specifications.
        let prompt = if prompt.len() > 64 * 1024 {
            format!(
                "You are Autono's {role:?} provider. Read the complete task instructions from {} before working. Return the structured result required by the supplied JSON schema. Follow the workspace and role restrictions in that file.",
                prompt_path.display()
            )
        } else {
            prompt
        };
        let request = AgentRequest {
            config: config.clone(),
            role,
            cwd: cwd.clone(),
            prompt,
            schema_path,
            resume,
        };
        let response = async {
            let command = self
                .agents
                .prepare(&request, &self.runner, &self.stop)
                .await?;
            let process = self
                .runner
                .start(
                    command,
                    &cwd,
                    config.timeout_seconds,
                    session.process_key,
                    &self.stop,
                )
                .await?;
            session.process_run_id = Some(process.id);
            let checkpoint = self
                .journal
                .change(
                    "agent_process",
                    &format!("{role:?} process={}", process.id),
                    |r| set_session(r, &target, session.clone()),
                )
                .await;
            if checkpoint.is_err() {
                self.stop.cancel();
            }
            let output = self.runner.wait(process.id, &self.stop).await?;
            checkpoint?;
            self.agents.decode(config.provider, &output)
        }
        .await;
        match &response {
            Ok(response) => {
                session.provider_session_id = response.session_id.clone();
                session.result = Some(serde_json::to_string(&response.result)?);
                session.outcome = Some("succeeded".into());
            }
            Err(error) => {
                session.outcome = Some(
                    error
                        .downcast_ref::<AgentFailure>()
                        .map(|e| e.kind.as_str())
                        .unwrap_or("failed")
                        .into(),
                );
                session.result = Some(format!("{error:#}").chars().take(4000).collect());
            }
        }
        self.journal
            .change(
                "agent_finished",
                &format!(
                    "{:?} session={} outcome={}",
                    role,
                    session.id,
                    session.outcome.as_deref().unwrap_or("unknown")
                ),
                |r| set_session(r, &target, session.clone()),
            )
            .await?;
        response
    }
    pub async fn execute(&self) -> Result<()> {
        let mut jobs = JoinSet::new();
        let parallel = self.journal.read().await.settings.execution.max_parallel;
        let mut fatal = None;
        loop {
            if !self.stop.is_cancelled() && fatal.is_none() {
                let scheduled: Result<()> = async {
                    // Dependency failure propagates only to descendants; independent work proceeds.
                    self.journal
                        .change("scheduler", "Dependency states evaluated", |r| {
                            loop {
                                let blocked: Vec<_> = r
                                    .tasks
                                    .iter()
                                    .filter(|t| {
                                        matches!(
                                            t.status,
                                            TaskStatus::Failed
                                                | TaskStatus::Blocked
                                                | TaskStatus::Cancelled
                                                | TaskStatus::Interrupted
                                        )
                                    })
                                    .map(|t| t.plan.id.clone())
                                    .collect();
                                let mut changed = false;
                                for t in &mut r.tasks {
                                    if matches!(t.status, TaskStatus::Waiting | TaskStatus::Ready)
                                        && t.plan.dependencies.iter().any(|d| blocked.contains(d))
                                    {
                                        changed = true;
                                        t.status = TaskStatus::Blocked;
                                        r.blockers.push(Blocker {
                                            task_id: Some(t.plan.id.clone()),
                                            kind: "dependency".into(),
                                            detail: "A required dependency did not complete".into(),
                                            action: "Resolve the dependency in a new run".into(),
                                            created_at: Utc::now(),
                                        });
                                    }
                                }
                                if !changed {
                                    break;
                                }
                            }
                            Ok(())
                        })
                        .await?;
                    let ready = self.journal.read().await.ready_tasks();
                    for id in ready.into_iter().take(parallel.saturating_sub(jobs.len())) {
                        let current = self.journal.read().await;
                        let w = workspace::reference(
                            &self.home,
                            current.id,
                            &format!("task-{id}"),
                            &current.base_commit,
                        );
                        self.journal
                            .change("task_started", &id, |r| {
                                let t = task_mut(r, &id)?;
                                if t.status == TaskStatus::Waiting {
                                    t.status = TaskStatus::Ready;
                                }
                                Ok(())
                            })
                            .await?;
                        self.journal
                            .change(
                                "workspace_reserved",
                                &format!("{id}: {} ({})", w.path.display(), w.branch),
                                |r| {
                                    let t = task_mut(r, &id)?;
                                    t.status = TaskStatus::Running;
                                    t.workspace = Some(w);
                                    Ok(())
                                },
                            )
                            .await?;
                        let this = self.clone();
                        jobs.spawn(async move {
                            let result = this.run_task(&id).await;
                            (id, result)
                        });
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = scheduled {
                    fatal = Some(error);
                    self.stop.cancel();
                }
            }
            if jobs.is_empty() {
                break;
            }
            match jobs.join_next().await {
                Some(Ok((id, Err(error)))) => {
                    if !self.stop.is_cancelled()
                        && let Err(save_error) = self.fail_task(&id, &error).await
                    {
                        fatal = Some(save_error);
                        self.stop.cancel();
                    }
                }
                Some(Err(error)) => {
                    fatal = Some(anyhow::anyhow!("task worker panicked: {error}"));
                    self.stop.cancel();
                }
                _ => {}
            }
        }
        if let Some(error) = fatal {
            return Err(error);
        }
        if self.stop.is_cancelled() {
            anyhow::bail!("workflow stopped");
        }
        self.journal.change("execution_finished","All runnable tasks finished",|r| {
            r.status = if r.tasks.iter().all(|t|t.status==TaskStatus::Completed) {EngineeringStatus::Verified} else if r.tasks.iter().any(|t|t.status==TaskStatus::Blocked) {EngineeringStatus::Blocked} else {EngineeringStatus::Failed};
            r.summary = format!("{} of {} tasks verified and independently accepted. {}",r.tasks.iter().filter(|t|t.status==TaskStatus::Completed).count(),r.tasks.len(),if r.status==EngineeringStatus::Verified {"Artifacts retained; run integrate for combined checks in a local integration branch."} else {"Inspect attempts and blockers; all workspaces retained."}); Ok(())
        }).await
    }
    async fn fail_task(&self, id: &str, error: &anyhow::Error) -> Result<()> {
        let blocked = error
            .downcast_ref::<AgentFailure>()
            .is_some_and(|e| e.blocked);
        let kind = error
            .downcast_ref::<AgentFailure>()
            .map(|e| e.kind.as_str())
            .unwrap_or("task_failure");
        let detail = format!("{error:#}").chars().take(6000).collect::<String>();
        self.journal.change("task_stopped",&format!("{id}: {detail}"),|r| {
            let t=task_mut(r,id)?;t.status=if blocked {TaskStatus::Blocked}else{TaskStatus::Failed};
            if let Some(a)=t.attempts.last_mut(){a.detail=Some(detail.clone());}
            if blocked {r.blockers.push(Blocker{task_id:Some(id.into()),kind:kind.into(),detail:detail.clone(),action:"Inspect the retained workspace and process logs; resolve the manual prerequisite before submitting a new plan".into(),created_at:Utc::now()});}Ok(())
        }).await
    }
    async fn run_task(&self, id: &str) -> Result<()> {
        let run = self.journal.read().await;
        let t = run
            .tasks
            .iter()
            .find(|t| t.plan.id == id)
            .context("missing task")?;
        let task = t.plan.clone();
        let mut w = t.workspace.clone().context("workspace not reserved")?;
        let _lease = WorkspaceLease::acquire(&w).map_err(workspace_blocker)?;
        workspace::create(&self.runner, &run.repository, &w, &self.stop)
            .await
            .map_err(workspace_blocker)?;
        for dependency in &task.dependencies {
            let dep = run
                .tasks
                .iter()
                .find(|t| &t.plan.id == dependency)
                .context("missing dependency")?;
            let commit = dep
                .artifact_commit
                .as_ref()
                .context("dependency has no artifact")?;
            if let Err(error) = workspace::merge(&self.runner, &w, commit, &self.stop).await {
                return Err(AgentFailure {
                    kind: "dependency_merge_conflict".into(),
                    detail: format!("{error:#}"),
                    blocked: true,
                }
                .into());
            }
        }
        w.base_commit = workspace::head(&self.runner, &w.path, &self.stop).await?;
        self.journal
            .change("workspace_ready", id, |r| {
                task_mut(r, id)?.workspace = Some(w.clone());
                Ok(())
            })
            .await?;
        let mut feedback = String::new();
        let mut resume = None;
        let mut review_count = 0;
        for number in 1..=run.settings.execution.max_attempts {
            ensure!(!self.stop.is_cancelled(), "workflow stopped");
            let session = new_session(&run.settings.agents.coder, AgentRole::Coder, resume.clone());
            self.journal
                .change("attempt_started", &format!("{id}: attempt {number}"), |r| {
                    task_mut(r, id)?.attempts.push(TaskAttempt {
                        number,
                        coder: session,
                        verification: vec![],
                        reviewer: None,
                        review: None,
                        snapshot: None,
                        detail: None,
                    });
                    Ok(())
                })
                .await?;
            let prompt = format!(
                "You implement one Autono engineering task. Work only in {}. Do not commit, change branches, edit Git metadata, spawn agents, or change other checkouts. The harness owns verification, commits and completion. Return summary and blocker (empty string if none). If permission, credentials or manual input are required, report a blocker and stop.\nOriginal feature specification:\n{}\nTask:\n{}\nCompleted dependencies (available in this checkout): {}\nArchitecture context:\n{}\nFeedback from the previous deterministic checks / independent review:\n{}",
                w.path.display(),
                run.specification,
                serde_json::to_string_pretty(&task)?,
                serde_json::to_string(&task.dependencies)?,
                context(&w.path).await,
                feedback
            );
            let coder = self
                .call_agent(
                    SessionTarget::Coder(id.into(), number),
                    run.settings.agents.coder.clone(),
                    AgentRole::Coder,
                    w.path.clone(),
                    prompt,
                    resume.clone(),
                )
                .await;
            workspace::assert_owned(&self.runner, &w, &w.base_commit, &self.stop).await?;
            match coder {
                Ok(response) => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct CodingResult {
                        summary: String,
                        blocker: String,
                    }
                    let result: CodingResult =
                        serde_json::from_value(response.result).context("invalid coder result")?;
                    ensure!(
                        !result.summary.trim().is_empty(),
                        "coder returned no implementation summary"
                    );
                    if !result.blocker.is_empty() {
                        return Err(AgentFailure {
                            kind: "human_action".into(),
                            detail: result.blocker,
                            blocked: true,
                        }
                        .into());
                    }
                    resume = response.session_id;
                }
                Err(error) => {
                    if error
                        .downcast_ref::<AgentFailure>()
                        .is_some_and(|e| e.blocked)
                    {
                        return Err(error);
                    }
                    feedback = format!("Coder invocation failed: {error:#}");
                    self.attempt_detail(id, number, &feedback).await?;
                    continue;
                }
            }
            workspace::stage(&self.runner, &w.path, &self.stop).await?;
            let snapshot = workspace::snapshot(&self.runner, &w.path, &self.stop).await?;
            let checks = self.checks(Some((id, number)), &w.path).await?;
            let after = workspace::snapshot(&self.runner, &w.path, &self.stop).await?;
            if after != snapshot {
                return Err(AgentFailure{kind:"verification_mutation".into(),detail:"Verification changed tracked or nonignored workspace contents; refusing stale evidence".into(),blocked:true}.into());
            }
            if checks.iter().any(|c| !c.passed) {
                feedback = format!(
                    "Required verification failed. Fix the implementation without weakening checks:\n{}",
                    serde_json::to_string(&checks)?
                );
                self.attempt_detail(id, number, &feedback).await?;
                continue;
            }
            self.journal
                .change("verification_passed", id, |r| {
                    task_mut(r, id)?.attempts[number - 1].snapshot = Some(snapshot.clone());
                    Ok(())
                })
                .await?;
            review_count += 1;
            let diff = self
                .runner
                .git(
                    &w.path,
                    &["diff", "--cached", "--binary", &w.base_commit],
                    &self.stop,
                )
                .await?;
            let artifact = self
                .home
                .join("engineering")
                .join(run.id.to_string())
                .join("artifacts")
                .join(format!("{id}-{number}.diff"));
            tokio::fs::write(&artifact, &diff).await?;
            let excerpt: String = diff.chars().take(48_000).collect();
            let prompt = format!(
                "You are an independent read-only reviewer in a NEW session. Inspect the code and diff, not coder reasoning. Do not edit files, commit, run commands, spawn agents, or alter Git state. Decide whether every acceptance criterion is satisfied and give concrete evidence for each criterion using its exact text. Findings need severity (critical/high/medium block completion), file, line (0 if unavailable), explanation, recommended_fix and confidence 0..1. Accept only if all criteria are satisfied with no blocking findings. Use blocker for missing manual prerequisites.\nOriginal feature specification:\n{}\nOriginal task:\n{}\nDeterministic verification evidence (full logs retained by process ID):\n{}\nProject context:\n{}\nDiff against task base {} (full diff at {}):\n{}{}",
                run.specification,
                serde_json::to_string_pretty(&task)?,
                serde_json::to_string(&checks)?,
                context(&w.path).await,
                w.base_commit,
                artifact.display(),
                excerpt,
                if excerpt.len() < diff.len() {
                    "\n[Excerpt truncated. Read the full diff artifact and affected source files.]"
                } else {
                    ""
                }
            );
            let response = self
                .call_agent(
                    SessionTarget::Reviewer(id.into(), number),
                    run.settings.agents.reviewer.clone(),
                    AgentRole::Reviewer,
                    w.path.clone(),
                    prompt,
                    None,
                )
                .await;
            if workspace::snapshot(&self.runner, &w.path, &self.stop).await? != snapshot {
                return Err(AgentFailure {
                    kind: "reviewer_mutation".into(),
                    detail: "Reviewer changed workspace state; review rejected, workspace retained"
                        .into(),
                    blocked: true,
                }
                .into());
            }
            let review: ReviewResult = match response {
                Ok(r) => serde_json::from_value(r.result).context("invalid structured review")?,
                Err(e) => {
                    if e.downcast_ref::<AgentFailure>().is_some_and(|e| e.blocked) {
                        return Err(e);
                    }
                    feedback = format!("Independent review invocation failed: {e:#}");
                    if review_count >= run.settings.execution.max_review_cycles {
                        return Err(e);
                    }
                    self.attempt_detail(id, number, &feedback).await?;
                    continue;
                }
            };
            review.validate(&task)?;
            self.journal
                .change(
                    "review_finished",
                    &format!(
                        "{id}: accepted={} findings={}",
                        review.accepts(),
                        review.findings.len()
                    ),
                    |r| {
                        task_mut(r, id)?.attempts[number - 1].review = Some(review.clone());
                        Ok(())
                    },
                )
                .await?;
            if !review.blocker.is_empty() {
                return Err(AgentFailure {
                    kind: "review_blocker".into(),
                    detail: review.blocker,
                    blocked: true,
                }
                .into());
            }
            if !review.accepts() {
                feedback = serde_json::to_string_pretty(&review)?;
                self.attempt_detail(id, number, &feedback).await?;
                ensure!(
                    review_count < run.settings.execution.max_review_cycles,
                    "review cycle limit exhausted for {id}"
                );
                continue;
            }
            workspace::assert_owned(&self.runner, &w, &w.base_commit, &self.stop).await?;
            let commit = workspace::commit(
                &self.runner,
                &w.path,
                &format!("Autono {}: {}", id, task.goal),
                &self.stop,
            )
            .await?;
            ensure!(
                self.runner
                    .git(&w.path, &["status", "--porcelain"], &self.stop)
                    .await?
                    .is_empty(),
                "workspace changed during artifact preservation"
            );
            self.journal
                .change("task_completed", &format!("{id}: {commit}"), |r| {
                    let t = task_mut(r, id)?;
                    t.status = TaskStatus::Completed;
                    t.artifact_commit = Some(commit.clone());
                    t.workspace
                        .as_mut()
                        .context("workspace missing")?
                        .head_commit = Some(commit);
                    Ok(())
                })
                .await?;
            return Ok(());
        }
        anyhow::bail!("coding attempt limit exhausted for {id}: {feedback}")
    }
    async fn attempt_detail(&self, id: &str, n: usize, detail: &str) -> Result<()> {
        self.journal
            .change("attempt_feedback", &format!("{id}: attempt {n}"), |r| {
                task_mut(r, id)?.attempts[n - 1].detail =
                    Some(detail.chars().take(24_000).collect());
                Ok(())
            })
            .await
    }
    async fn checks(
        &self,
        target: Option<(&str, usize)>,
        cwd: &std::path::Path,
    ) -> Result<Vec<VerificationResult>> {
        let run = self.journal.read().await;
        let mut checks = Vec::new();
        for command in &run.settings.verification.commands {
            let key = Uuid::new_v4();
            self.journal
                .change(
                    "verification_started",
                    &format!(
                        "target={target:?} process_key={key} command={}",
                        serde_json::to_string(command)?
                    ),
                    |_| Ok(()),
                )
                .await?;
            let process = self
                .runner
                .start(
                    command.clone(),
                    cwd,
                    run.settings.verification.timeout_seconds,
                    key,
                    &self.stop,
                )
                .await?;
            let checkpoint = self
                .journal
                .change(
                    "verification_process",
                    &format!("target={target:?} process={}", process.id),
                    |_| Ok(()),
                )
                .await;
            if checkpoint.is_err() {
                self.stop.cancel();
            }
            let output = self.runner.wait(process.id, &self.stop).await?;
            checkpoint?;
            let check = VerificationResult {
                process_key: key,
                process_run_id: output.run.id,
                command: command.clone(),
                exit_code: output.run.exit_code,
                passed: output.succeeded(),
                output_complete: output.run.output_complete == Some(true),
                summary: output.summary(),
            };
            self.journal
                .change(
                    "verification_finished",
                    &format!(
                        "target={target:?} process={} passed={}",
                        check.process_run_id, check.passed
                    ),
                    |r| {
                        if let Some((id, n)) = target {
                            task_mut(r, id)?.attempts[n - 1]
                                .verification
                                .push(check.clone());
                        } else {
                            r.integration_verification.push(check.clone());
                        }
                        Ok(())
                    },
                )
                .await?;
            checks.push(check);
        }
        Ok(checks)
    }
    pub async fn integrate(&self) -> Result<()> {
        let run = self.journal.read().await;
        let w = run
            .integration_workspace
            .clone()
            .context("integration workspace not reserved")?;
        let _lease = WorkspaceLease::acquire(&w).map_err(workspace_blocker)?;
        workspace::create(&self.runner, &run.repository, &w, &self.stop)
            .await
            .map_err(workspace_blocker)?;
        for id in run.plan.as_ref().context("missing plan")?.validate()? {
            let task = run
                .tasks
                .iter()
                .find(|t| t.plan.id == id)
                .context("missing task")?;
            let owned = task.workspace.as_ref().context("missing task workspace")?;
            let _task_lease = WorkspaceLease::acquire(owned).map_err(workspace_blocker)?;
            let commit = task.artifact_commit.as_ref().context("missing artifact")?;
            workspace::assert_owned(&self.runner, owned, commit, &self.stop).await?;
            ensure!(
                self.runner
                    .git(&owned.path, &["status", "--porcelain"], &self.stop)
                    .await?
                    .is_empty(),
                "completed task workspace is dirty"
            );
            if let Err(error) = workspace::merge(&self.runner, &w, commit, &self.stop).await {
                return Err(AgentFailure {
                    kind: "integration_conflict".into(),
                    detail: format!(
                        "{id}: {error:#}; conflict state retained at {}",
                        w.path.display()
                    ),
                    blocked: true,
                }
                .into());
            }
            self.journal
                .change("integration_merge", &format!("{id}: {commit}"), |_| Ok(()))
                .await?;
        }
        let snapshot = workspace::snapshot(&self.runner, &w.path, &self.stop).await?;
        let checks = self.checks(None, &w.path).await?;
        ensure!(
            checks.iter().all(|c| c.passed),
            "combined integration verification failed; inspect integration process logs"
        );
        ensure!(
            snapshot == workspace::snapshot(&self.runner, &w.path, &self.stop).await?,
            "combined verification mutated integration workspace"
        );
        let commit = workspace::head(&self.runner, &w.path, &self.stop).await?;
        self.journal.change("completed",&format!("Verified integration commit {commit}"),|r|{r.integration_workspace.as_mut().context("missing workspace")?.head_commit=Some(commit.clone());r.status=EngineeringStatus::Completed;r.summary=format!("All {} tasks passed deterministic checks and independent review. Combined checks passed. Local integration branch {} at {} ({}). No merge to main was performed.",r.tasks.len(),w.branch,commit,w.path.display());Ok(())}).await
    }
}
async fn context(path: &std::path::Path) -> String {
    let mut text = String::new();
    let Ok(root) = tokio::fs::canonicalize(path).await else {
        return text;
    };
    for name in [
        "README.md",
        "AGENTS.md",
        "docs/architecture.md",
        "docs/runtime.md",
    ] {
        // Do not follow repository context symlinks into the user's home/credentials.
        let Ok(file) = tokio::fs::canonicalize(root.join(name)).await else {
            continue;
        };
        if !file.starts_with(&root) {
            continue;
        }
        if let Ok(bytes) = tokio::fs::read(file).await {
            let content = String::from_utf8_lossy(&bytes);
            text.push_str(&format!(
                "\n{name}:\n{}\n",
                content.chars().take(6000).collect::<String>()
            ));
        }
    }
    text
}

fn workspace_blocker(error: anyhow::Error) -> anyhow::Error {
    AgentFailure {
        kind: "workspace_prerequisite".into(),
        detail: format!("{error:#}"),
        blocked: true,
    }
    .into()
}

#[cfg(all(test, unix))]
mod context_tests {
    #[tokio::test]
    async fn context_does_not_follow_symlinks_outside_checkout() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let checkout = dir.path().join("checkout");
        tokio::fs::create_dir_all(checkout.join("docs")).await?;
        let outside = dir.path().join("private.txt");
        tokio::fs::write(&outside, "FAKE_CREDENTIAL_SENTINEL").await?;
        std::os::unix::fs::symlink(outside, checkout.join("README.md"))?;
        tokio::fs::write(checkout.join("docs/runtime.md"), "Safe project context").await?;
        let context = super::context(&checkout).await;
        assert!(context.contains("Safe project context"));
        assert!(!context.contains("FAKE_CREDENTIAL_SENTINEL"));
        Ok(())
    }
}
