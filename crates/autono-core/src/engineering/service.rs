use super::{
    agent::{AgentFailure, AgentRuntime, CliAgentRuntime},
    engine::Engine,
    journal::Journal,
    process::ProcessRunner,
    workspace, *,
};
use crate::{Supervisor, config::Config, persistence::retry};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

#[derive(Clone)]
pub struct EngineeringService {
    inner: Arc<Inner>,
}
struct Inner {
    runner: ProcessRunner,
    config: Config,
    home: PathBuf,
    agents: Arc<dyn AgentRuntime>,
    registry: Mutex<Registry>,
    jobs: TaskTracker,
    shutdown: CancellationToken,
    control: tokio::sync::Mutex<()>,
    wakeup: Arc<tokio::sync::Notify>,
    mutations: Arc<tokio::sync::Semaphore>,
}
struct Registry {
    accepting: bool,
    active: Option<(Uuid, CancellationToken)>,
    error: Option<String>,
    journal: Option<Journal>,
}
enum Operation {
    Plan(Box<PlanSubmission>),
    Execute(Uuid),
    Preflight(Uuid),
    Continue(Uuid),
    Integrate(Uuid),
}
impl Operation {
    fn id(&self) -> Uuid {
        match self {
            Self::Plan(s) => s.id,
            Self::Execute(id) | Self::Integrate(id) | Self::Preflight(id) | Self::Continue(id) => {
                *id
            }
        }
    }
}

impl EngineeringService {
    pub fn new(supervisor: Supervisor, home: PathBuf, config: Config) -> Self {
        Self::with_agents(supervisor, home, config, Arc::new(CliAgentRuntime))
    }
    /// Dependency injection keeps all CI orchestration deterministic and inference-free.
    pub fn with_agents(
        supervisor: Supervisor,
        home: PathBuf,
        config: Config,
        agents: Arc<dyn AgentRuntime>,
    ) -> Self {
        let mutations = Arc::new(tokio::sync::Semaphore::new(
            config.runner.max_pending_submissions,
        ));
        Self {
            inner: Arc::new(Inner {
                runner: ProcessRunner::new(supervisor),
                config,
                home,
                agents,
                registry: Mutex::new(Registry {
                    accepting: true,
                    active: None,
                    error: None,
                    journal: None,
                }),
                jobs: TaskTracker::new(),
                shutdown: CancellationToken::new(),
                control: tokio::sync::Mutex::new(()),
                wakeup: Arc::new(tokio::sync::Notify::new()),
                mutations,
            }),
        }
    }
    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn store(&self) -> &crate::Store {
        self.inner.runner.supervisor.store()
    }
    pub fn health(&self) -> (Option<Uuid>, Option<String>) {
        let r = self.registry();
        (r.active.as_ref().map(|(id, _)| *id), r.error.clone())
    }
    pub async fn plan(&self, submission: PlanSubmission) -> Result<EngineeringRun> {
        ensure!(
            submission.repository.is_absolute()
                && !submission.specification.trim().is_empty()
                && submission.specification.len() <= 128 * 1024,
            "specification must be 1..128 KiB with an absolute repository path"
        );
        // Explicit IDs are submission keys: retry never silently starts another planner.
        if let Ok(existing) = self.store().engineering_get(submission.id).await {
            ensure!(
                existing.specification == submission.specification
                    && existing.repository == submission.repository
                    && existing.requested_base == submission.base,
                EngineeringConflict(
                    "plan ID already has a different specification, repository or base".into()
                )
            );
            return Ok(existing);
        }
        self.launch(Operation::Plan(Box::new(submission))).await
    }
    pub async fn execute(&self, id: Uuid) -> Result<EngineeringRun> {
        ensure!(
            self.store().engineering_get(id).await?.status == EngineeringStatus::AwaitingApproval,
            EngineeringConflict("execute requires AWAITING_APPROVAL".into())
        );
        self.launch(Operation::Execute(id)).await
    }
    pub async fn preflight(&self, id: Uuid) -> Result<EngineeringRun> {
        self.launch(Operation::Preflight(id)).await
    }
    pub async fn continue_run(&self, id: Uuid) -> Result<EngineeringRun> {
        {
            let _gate = self.inner.control.lock().await;
            let active = {
                let registry = self.registry();
                registry
                    .active
                    .as_ref()
                    .filter(|(active, token)| *active == id && !token.is_cancelled())
                    .and(registry.journal.clone())
            };
            if let Some(journal) = active {
                journal
                    .change(
                        "continued",
                        "Developer continued eligible live pauses alongside independent workers",
                        |r| {
                            // Serialize this check with the scheduler's final checkpoint.
                            // Once drained, a new owned execution must be admitted instead.
                            ensure!(r.status == EngineeringStatus::Running, EngineeringConflict("scheduler has drained; retry continue after the run becomes inactive".into()));
                            mark_continued(r)
                        },
                    )
                    .await?;
                self.inner.wakeup.notify_one();
                return Ok(journal.read().await);
            }
        }
        self.launch(Operation::Continue(id)).await
    }
    pub async fn resolve(
        &self,
        id: Uuid,
        action_id: &str,
        request: ResolutionRequest,
    ) -> Result<EngineeringRun> {
        // Registration is synchronous with shutdown admission. The HTTP future
        // owns only the reply receiver, never the commit/publication lifecycle.
        let (reply, response) = oneshot::channel();
        let action_id = action_id.to_owned();
        if !self.registry().accepting {
            if let Some(result) = self
                .store()
                .engineering_resolution_result(id, &action_id, &request)
                .await?
            {
                return Ok(result);
            }
            return Err(crate::supervisor::ShuttingDown.into());
        }
        {
            let registry = self.registry();
            ensure!(registry.accepting, crate::supervisor::ShuttingDown);
            ensure!(
                registry.error.is_none(),
                EngineeringConflict("workflow persistence is degraded".into())
            );
            let permit = self
                .inner
                .mutations
                .clone()
                .try_acquire_owned()
                .map_err(|_| crate::supervisor::Overloaded("pending engineering mutations"))?;
            let this = self.clone();
            self.inner.jobs.spawn(async move {
                let owner = this.clone();
                let job = tokio::spawn(async move { owner.apply_resolution(id, &action_id, request).await });
                let result = match job.await {
                    Ok(result) => result,
                    Err(_) => {
                        this.registry().error = Some("Resolution controller stopped before confirming publication; inspect durable receipts and restart reconciliation".into());
                        this.signal_shutdown();
                        Err(anyhow::anyhow!("resolution publication is uncertain; workflow stopped safely"))
                    }
                };
                let _ = reply.send(result);
                drop(permit);
            });
        }
        response
            .await
            .context("resolution controller stopped before responding")?
    }
    async fn apply_resolution(
        &self,
        id: Uuid,
        action_id: &str,
        request: ResolutionRequest,
    ) -> Result<EngineeringRun> {
        // One gate for inactive mutations and launch preparation. Active workers use
        // the same journal, so API requests cannot overwrite scheduler checkpoints.
        let _gate = self.inner.control.lock().await;
        let journal = {
            let registry = self.registry();
            registry
                .active
                .as_ref()
                .filter(|(active, _)| *active == id)
                .and(registry.journal.clone())
        };
        let journal = match journal {
            Some(j) => j,
            None => Journal::new(
                self.store().clone(),
                self.store().engineering_get(id).await?,
            ),
        };
        if let Some(result) = self
            .store()
            .engineering_resolution_result(id, action_id, &request)
            .await?
        {
            // Even an ambiguous earlier commit must be published before returning.
            self.reconcile_resolution_journal(&journal).await?;
            return Ok(result);
        }
        super::preflight::safe_text(&request.note)?;
        let receipt_request = request.clone();
        let committed = journal.change("human_resolution", "Explicit developer resolution recorded; no operation was executed", |r| {
            ensure!(matches!(r.status, EngineeringStatus::AwaitingApproval | EngineeringStatus::Running | EngineeringStatus::Blocked), EngineeringConflict("cannot resolve a terminal or interrupted workflow".into()));
            let action = r.human_actions.iter_mut().find(|a| a.id == action_id).context("unknown human action")?;
            ensure!(action.status != HumanActionStatus::Resolved, EngineeringConflict("action already resolved; reuse the original request key".into()));
            match request.disposition {
                ResolutionDisposition::Approve => {
                    let permission = action.requirement.permission.as_ref().context("action is not a permission request")?;
                    ensure!(request.permission.as_ref() == Some(permission), EngineeringConflict("approval must identify the exact requested operation and scope".into()));
                    ensure!(r.settings.permissions.classify(permission.operation) == PermissionRule::RequireApproval, EngineeringConflict("policy does not permit approval".into()));
                    ensure!(permission.supported(r.id), EngineeringConflict("adapter cannot safely grant this operation; configure it externally, retain the blocker, or use a new trusted base".into()));
                }
                ResolutionDisposition::Resolve => {
                    ensure!(action.requirement.permission.is_none() && request.permission.is_none(), EngineeringConflict("permission requires an explicit approval, never a textual resolution".into()));
                }
                _ => { ensure!(request.permission.is_none(), "defer/reject must not carry an approval"); }
            }
            if matches!(request.disposition, ResolutionDisposition::Approve | ResolutionDisposition::Resolve) && action.continuable {
                ensure!(request.effect != EffectState::Unknown, EngineeringConflict("inspect operation effects before continuation".into()));
            }
            action.status = match request.disposition { ResolutionDisposition::Resolve | ResolutionDisposition::Approve => HumanActionStatus::Resolved, ResolutionDisposition::Defer => HumanActionStatus::Deferred, ResolutionDisposition::Reject => HumanActionStatus::Rejected };
            action.resolution = Some(request.clone());
            action.updated_at = Utc::now();
            action.audit.push(ActionAudit { at: action.updated_at, status: action.status, request: Some(request) });
            Ok(())
        }).await;
        if let Err(error) = committed {
            // A commit error is not proof of rollback. Confirm the durable state
            // while holding the mutation gate, publish it, then consult the receipt.
            self.reconcile_resolution_journal(&journal).await?;
            if let Some(result) = self
                .store()
                .engineering_resolution_result(id, action_id, &receipt_request)
                .await?
            {
                return Ok(result);
            }
            return Err(error);
        }
        self.store()
            .engineering_resolution_result(id, action_id, &receipt_request)
            .await?
            .context("committed resolution receipt missing")
    }
    async fn reconcile_resolution_journal(&self, journal: &Journal) -> Result<()> {
        if let Err(error) = journal.reconcile().await {
            self.registry().error = Some("Cannot establish durable resolution state; accepted mutation reconciliation failed. Inspect receipts and restart after storage repair.".into());
            self.signal_shutdown();
            return Err(error.context("resolution state uncertain; workflow stopped safely"));
        }
        Ok(())
    }
    pub async fn integrate(&self, id: Uuid) -> Result<EngineeringRun> {
        ensure!(
            self.store().engineering_get(id).await?.status == EngineeringStatus::Verified,
            EngineeringConflict("integrate requires VERIFIED tasks".into())
        );
        self.launch(Operation::Integrate(id)).await
    }
    async fn launch(&self, op: Operation) -> Result<EngineeringRun> {
        let id = op.id();
        let stop = CancellationToken::new();
        let (reply, response) = oneshot::channel();
        {
            let mut r = self.registry();
            ensure!(r.accepting, crate::supervisor::ShuttingDown);
            ensure!(r.error.is_none(),EngineeringConflict("workflow persistence is degraded; inspect daemon logs and restart after repair".into()));
            ensure!(
                r.active.is_none(),
                crate::supervisor::Overloaded("one active engineering run per daemon")
            );
            r.active = Some((id, stop.clone()));
            let this = self.clone();
            // This registered task owns preparation, durable acceptance, and execution.
            // Losing the HTTP response/CLI waiter cannot cancel the workflow.
            self.inner.jobs.spawn(async move {
                this.run_owned(op, stop, reply).await;
            });
        }
        response.await.context("workflow admission task stopped")?
    }
    async fn run_owned(
        &self,
        op: Operation,
        stop: CancellationToken,
        reply: oneshot::Sender<Result<EngineeringRun>>,
    ) {
        let id = op.id();
        let owner = self.clone();
        let signal = stop.clone();
        let job = tokio::spawn(async move { owner.operate(op, signal, reply).await });
        let result = match job.await {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!("workflow controller panicked: {error}")),
        };
        if let Err(error) = result {
            let externally_cancelled = stop.is_cancelled();
            stop.cancel();
            tracing::error!(%id,error=%format!("{error:#}"),"engineering workflow stopped");
            if let Err(error) = self.record_failure(id, &error, externally_cancelled).await {
                self.registry().error = Some(format!("run {id}: {error:#}"));
            }
        }
        let mut registry = self.registry();
        registry.active = None;
        registry.journal = None;
    }
    async fn record_failure(
        &self,
        id: Uuid,
        error: &anyhow::Error,
        externally_cancelled: bool,
    ) -> Result<()> {
        let _gate = self.inner.control.lock().await;
        let run = match self.store().engineering_get(id).await {
            Ok(run) => run,
            Err(e) if e.is::<EngineeringNotFound>() => return Ok(()),
            Err(e) => return Err(e),
        };
        if !run.status.active() {
            if externally_cancelled
                && !self.inner.shutdown.is_cancelled()
                && matches!(
                    run.status,
                    EngineeringStatus::AwaitingApproval
                        | EngineeringStatus::Verified
                        | EngineeringStatus::Blocked
                )
            {
                let journal = self
                    .registry()
                    .journal
                    .clone()
                    .unwrap_or_else(|| Journal::new(self.store().clone(), run));
                journal
                    .change(
                        "cancelled",
                        "Developer cancelled during workflow preparation; no work will be launched",
                        |r| {
                            r.status = EngineeringStatus::Cancelled;
                            for task in &mut r.tasks {
                                if matches!(
                                    task.status,
                                    TaskStatus::Waiting
                                        | TaskStatus::Ready
                                        | TaskStatus::BlockedHuman
                                ) {
                                    task.status = TaskStatus::Cancelled;
                                }
                            }
                            r.summary = "Cancelled by developer; artifacts retained".into();
                            Ok(())
                        },
                    )
                    .await?;
            }
            return Ok(());
        }
        let journal = self
            .registry()
            .journal
            .clone()
            .unwrap_or_else(|| Journal::new(self.store().clone(), run));
        let shutdown = self.inner.shutdown.is_cancelled();
        let manual = error
            .downcast_ref::<AgentFailure>()
            .is_some_and(|e| e.blocked);
        let detail = format!("{error:#}").chars().take(6000).collect::<String>();
        journal.change("workflow_stopped",&detail,|r| {
            r.status=if shutdown {EngineeringStatus::Interrupted} else if manual {EngineeringStatus::Blocked} else if error.is::<UserCancelled>() {EngineeringStatus::Cancelled} else {EngineeringStatus::Failed};
            for task in &mut r.tasks {
                if matches!(task.status,TaskStatus::Running|TaskStatus::Ready|TaskStatus::Waiting|TaskStatus::BlockedHuman) {
                    task.status=if r.status==EngineeringStatus::Cancelled {TaskStatus::Cancelled} else {TaskStatus::Interrupted};
                }
            }
            r.summary=detail.clone();
            if manual {
                let requirement = HumanRequirement { kind: HumanActionType::Runtime, reason: "Workflow stage needs external intervention".into(), affected_tasks: vec![], requested_action: "Inspect the stage and configure the prerequisite externally. Planning/integration/integrity failures require a new trusted plan; they are not live task pauses.".into(), blocking: true, evidence: "Workflow stage failed; retained process IDs and workspaces identify its evidence".into(), confidence: 100, permission: None };
                r.human_actions.push(super::preflight::action(requirement, &format!("stage-{}-{}", r.id, r.revision), false, None));
            }
            if manual||shutdown||externally_cancelled {
                r.blockers.push(Blocker {task_id:None,kind:if manual {"manual_action"} else {"interrupted"}.into(),detail:detail.clone(),action:"Inspect retained workspaces and process logs before submitting a new plan".into(),created_at:Utc::now()});
            }
            Ok(())
        }).await
    }
    async fn operate(
        &self,
        op: Operation,
        stop: CancellationToken,
        reply: oneshot::Sender<Result<EngineeringRun>>,
    ) -> Result<()> {
        let prepared = {
            let _gate = self.inner.control.lock().await;
            let result = self.prepare(op, &stop).await;
            if let Ok((journal, _)) = &result {
                self.registry().journal = Some(journal.clone());
            }
            result
        };
        let (journal, operation) = match prepared {
            Ok(value) => value,
            Err(error) => {
                let diagnostic = format!("{error:#}");
                let _ = reply.send(Err(error));
                return Err(anyhow::anyhow!(diagnostic));
            }
        };
        let _ = reply.send(Ok(journal.read().await));
        let engine = Engine {
            journal,
            runner: self.inner.runner.clone(),
            agents: self.inner.agents.clone(),
            home: self.inner.home.clone(),
            stop: stop.child_token(),
            artifacts: Default::default(),
            trusted_context: Default::default(),
            wakeup: self.inner.wakeup.clone(),
        };
        let result = match operation {
            0 => engine.plan().await,
            1 => engine.execute().await,
            2 => engine.integrate().await,
            _ => Ok(()),
        };
        if stop.is_cancelled() && !self.inner.shutdown.is_cancelled() {
            return Err(UserCancelled.into());
        }
        result
    }
    async fn prepare(&self, op: Operation, stop: &CancellationToken) -> Result<(Journal, u8)> {
        match op {
            Operation::Plan(submission) => {
                let (repo, base) = workspace::repository(
                    &self.inner.runner,
                    &submission.repository,
                    &submission.base,
                    stop,
                )
                .await?;
                let config = self.inner.config.clone().with_project(&repo).await?;
                ensure!(
                    config.execution.max_parallel
                        <= self.inner.runner.supervisor.admission().max_active_runs,
                    "max_parallel exceeds runtime max_active_runs; lower engineering parallelism or increase process capacity"
                );
                let w = workspace::reference(&self.inner.home, submission.id, "planner", &base);
                let now = Utc::now();
                let run = EngineeringRun {
                    id: submission.id,
                    revision: 0,
                    status: EngineeringStatus::Planning,
                    created_at: now,
                    updated_at: now,
                    repository_git: Some(self.inner.runner.git_ownership(&repo)?),
                    repository: repo,
                    requested_base: submission.base,
                    base_commit: base,
                    specification: submission.specification,
                    settings: config.workflow(),
                    plan: None,
                    approved_at: None,
                    planner: None,
                    planning_workspace: w,
                    tasks: vec![],
                    blockers: vec![],
                    human_actions: vec![],
                    preflight: None,
                    integration_workspace: None,
                    integration_verification: vec![],
                    summary: "Planning requested".into(),
                };
                retry(|| self.store().engineering_create(&run)).await?;
                Ok((Journal::new(self.store().clone(), run), 0))
            }
            Operation::Execute(id) => {
                let run = self.store().engineering_get(id).await?;
                ensure!(
                    run.status == EngineeringStatus::AwaitingApproval,
                    EngineeringConflict(
                        "execute requires AWAITING_APPROVAL; runs are never automatically resumed"
                            .into()
                    )
                );
                run.plan.as_ref().context("plan missing")?.validate()?;
                ensure!(
                    run.settings.execution.max_parallel
                        <= self.inner.runner.supervisor.admission().max_active_runs,
                    "stored max_parallel exceeds runtime process capacity"
                );
                let journal = Journal::new(self.store().clone(), run);
                super::preflight::inspect(&journal, &self.inner.runner, &self.inner.agents, stop)
                    .await?;
                ensure!(
                    journal.read().await.preflight.is_some_and(|p| p.ready),
                    EngineeringConflict(
                        "preflight is NOT READY; inspect preflight and human actions".into()
                    )
                );
                ensure!(!stop.is_cancelled(), "execution cancelled during preflight");
                journal
                    .change(
                        "approved",
                        "Developer approved the persisted plan and configured checks",
                        |r| {
                            r.status = EngineeringStatus::Running;
                            r.approved_at = Some(Utc::now());
                            r.summary = "Executing approved plan".into();
                            Ok(())
                        },
                    )
                    .await?;
                Ok((journal, 1))
            }
            Operation::Preflight(id) => {
                let run = self.store().engineering_get(id).await?;
                ensure!(
                    run.plan.is_some()
                        && matches!(
                            run.status,
                            EngineeringStatus::AwaitingApproval | EngineeringStatus::Blocked
                        ),
                    EngineeringConflict("preflight requires a planned inactive run".into())
                );
                let journal = Journal::new(self.store().clone(), run);
                super::preflight::inspect(&journal, &self.inner.runner, &self.inner.agents, stop)
                    .await?;
                Ok((journal, 3))
            }
            Operation::Continue(id) => {
                let run = self.store().engineering_get(id).await?;
                ensure!(run.status == EngineeringStatus::Blocked && run.approved_at.is_some(), EngineeringConflict("continue requires a live-pause BLOCKED run; interrupted workflows cannot resume".into()));
                let journal = Journal::new(self.store().clone(), run);
                super::preflight::inspect(&journal, &self.inner.runner, &self.inner.agents, stop)
                    .await?;
                ensure!(
                    journal.read().await.preflight.is_some_and(|p| p.ready),
                    EngineeringConflict("preflight is NOT READY".into())
                );
                ensure!(!stop.is_cancelled(), "continuation cancelled");
                journal.change("continued", "Developer explicitly continued eligible live pauses; completed artifacts retained", mark_continued).await?;
                Ok((journal, 1))
            }
            Operation::Integrate(id) => {
                let run = self.store().engineering_get(id).await?;
                ensure!(
                    run.status == EngineeringStatus::Verified,
                    EngineeringConflict("integrate requires VERIFIED tasks".into())
                );
                let w = workspace::reference(&self.inner.home, id, "integration", &run.base_commit);
                let journal = Journal::new(self.store().clone(), run);
                journal
                    .change(
                        "integration_requested",
                        "Developer requested local integration and combined checks",
                        |r| {
                            r.status = EngineeringStatus::Integrating;
                            r.integration_workspace = Some(w);
                            Ok(())
                        },
                    )
                    .await?;
                Ok((journal, 2))
            }
        }
    }
    pub async fn cancel(&self, id: Uuid) -> Result<()> {
        let token = {
            let r = self.registry();
            r.active
                .as_ref()
                .filter(|(active, _)| *active == id)
                .map(|(_, token)| token.clone())
        };
        if let Some(token) = token {
            token.cancel();
            return Ok(());
        }
        let _gate = self.inner.control.lock().await;
        let run = self.store().engineering_get(id).await?;
        ensure!(
            matches!(
                run.status,
                EngineeringStatus::AwaitingApproval
                    | EngineeringStatus::Verified
                    | EngineeringStatus::Blocked
            ),
            EngineeringConflict("run is terminal or not owned by this daemon".into())
        );
        let journal = Journal::new(self.store().clone(), run);
        journal
            .change(
                "cancelled",
                "Developer cancelled the inactive workflow",
                |r| {
                    r.status = EngineeringStatus::Cancelled;
                    for t in &mut r.tasks {
                        if matches!(
                            t.status,
                            TaskStatus::Ready | TaskStatus::Waiting | TaskStatus::BlockedHuman
                        ) {
                            t.status = TaskStatus::Cancelled;
                        }
                    }
                    r.summary = "Cancelled by developer; artifacts retained".into();
                    Ok(())
                },
            )
            .await
    }
    pub fn signal_shutdown(&self) {
        let mut r = self.registry();
        r.accepting = false;
        self.inner.shutdown.cancel();
        if let Some((_, token)) = &r.active {
            token.cancel();
        }
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.signal_shutdown();
        self.inner.jobs.close();
        tokio::time::timeout(Duration::from_secs(15),self.inner.jobs.wait()).await.context("workflow checkpoints did not finish during shutdown; restart will reconcile unfinished state")?;
        ensure!(
            self.registry().error.is_none(),
            "workflow persistence failed during shutdown"
        );
        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
#[error("Workflow cancelled by developer; workspaces and logs retained")]
struct UserCancelled;

fn mark_continued(r: &mut EngineeringRun) -> Result<()> {
    ensure!(
        matches!(
            r.status,
            EngineeringStatus::Running | EngineeringStatus::Blocked
        ) && r.approved_at.is_some(),
        EngineeringConflict(
            "only approved live pauses can continue; interrupted workflows cannot resume".into()
        )
    );
    let mut eligible = vec![];
    for task in &r.tasks {
        if task.status != TaskStatus::BlockedHuman
            || task.attempts.len() >= r.settings.execution.max_attempts
        {
            continue;
        }
        let actions: Vec<_> = r
            .human_actions
            .iter()
            .filter(|a| {
                a.requirement.affected_tasks.contains(&task.plan.id)
                    && a.requirement.kind == HumanActionType::Runtime
            })
            .collect();
        if !actions.is_empty()
            && actions.iter().all(|a| {
                a.continuable
                    && a.status == HumanActionStatus::Resolved
                    && a.resolution
                        .as_ref()
                        .is_some_and(|q| q.effect != EffectState::Unknown)
            })
        {
            eligible.push(task.plan.id.clone());
        }
    }
    ensure!(
        !eligible.is_empty(),
        EngineeringConflict("no eligible resolved human-paused tasks".into())
    );
    for task in &mut r.tasks {
        if eligible.contains(&task.plan.id) {
            task.status = TaskStatus::Ready;
        }
    }
    r.status = EngineeringStatus::Running;
    r.summary = "Continuing explicitly resolved live pauses".into();
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::agent::{AgentFuture, AgentRequest, AgentResponse};
    use super::super::process::CommandOutput;
    use super::*;
    use crate::Store;
    use serde_json::json;

    struct Fake {
        root: PathBuf,
    }
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
    impl AgentRuntime for Fake {
        fn prepare<'a>(
            &'a self,
            request: &'a AgentRequest,
            _: &'a ProcessRunner,
            _: &'a CancellationToken,
        ) -> AgentFuture<'a> {
            Box::pin(async move {
                let mut prefix = String::new();
                let result = match request.role {
                    AgentRole::Planner => {
                        let tasks: Vec<_> = ["b", "c"].iter().map(|id| json!({"id":id,"goal":"Feature","description":"Implement behavior","acceptance_criteria":["Works"],"dependencies":[],"requirement_ids":["r"],"verification_expectations":["Checks pass"],"role":"coder"})).collect();
                        json!({"human_requirements":[],"requirements":[{"id":"r","text":"Feature"}],"milestones":[{"id":"m","goal":"Feature","task_ids":["b","c"]}],"tasks":tasks})
                    }
                    AgentRole::Coder => {
                        if request.cwd.ends_with("task-c") {
                            prefix = format!(
                                "touch {}; sleep 60; ",
                                quote(self.root.join("c-started").to_str().expect("path"))
                            );
                            json!({"summary":"Implemented","blocker":""})
                        } else {
                            json!({"summary":"Needs a decision","blocker":"Choose behavior before proceeding"})
                        }
                    }
                    AgentRole::Reviewer => anyhow::bail!("review must not run in this fixture"),
                };
                let events = match request.config.provider {
                    Provider::Claude => format!(
                        "{}\n",
                        json!({"type":"result","subtype":"success","session_id":Uuid::new_v4(),"structured_output":result})
                    ),
                    Provider::Codex => format!(
                        "{}\n{}\n{}\n",
                        json!({"type":"thread.started","thread_id":Uuid::new_v4()}),
                        json!({"type":"item.completed","item":{"type":"agent_message","text":result.to_string()}}),
                        json!({"type":"turn.completed"})
                    ),
                };
                Ok(vec![
                    "sh".into(),
                    "-c".into(),
                    format!("{prefix}printf '%s' {}", quote(&events)),
                ])
            })
        }
        fn decode(&self, provider: Provider, output: &CommandOutput) -> Result<AgentResponse> {
            CliAgentRuntime.decode(provider, output)
        }
    }
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        service: EngineeringService,
        supervisor: Supervisor,
        config: Config,
        id: Uuid,
    }
    impl Fixture {
        async fn planned(mut config: Config) -> Result<Self> {
            let dir = tempfile::tempdir()?;
            let root = tokio::fs::canonicalize(dir.path()).await?;
            let repository = root.join("repo");
            tokio::fs::create_dir(&repository).await?;
            let supervisor = Supervisor::new(Store::open(&root.join("db")).await?, false);
            let runner = ProcessRunner::new(supervisor.clone());
            let stop = CancellationToken::new();
            runner
                .git(&repository, &["init", "-b", "main"], &stop)
                .await?;
            tokio::fs::write(repository.join("README.md"), "Feature").await?;
            runner.git(&repository, &["add", "."], &stop).await?;
            runner
                .git(&repository, &["commit", "-m", "seed"], &stop)
                .await?;
            config.verification.commands = vec![vec!["true".into()]];
            config.agents.coder.timeout_seconds = 120;
            let service = EngineeringService::with_agents(
                supervisor.clone(),
                root.join("home"),
                config.clone(),
                Arc::new(Fake { root: root.clone() }),
            );
            let id = Uuid::new_v4();
            service
                .plan(PlanSubmission {
                    id,
                    repository,
                    base: "HEAD".into(),
                    specification: "Implement a feature".into(),
                })
                .await?;
            tokio::time::timeout(Duration::from_secs(60), async {
                loop {
                    if service.health().0.is_none() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await?;
            ensure!(
                service.store().engineering_get(id).await?.status
                    == EngineeringStatus::AwaitingApproval,
                "planning failed"
            );
            Ok(Self {
                _dir: dir,
                root,
                service,
                supervisor,
                config,
                id,
            })
        }
    }

    #[tokio::test]
    async fn cancelled_resolution_waiter_cannot_strand_commit_publication_or_shutdown() -> Result<()>
    {
        let f = Fixture::planned(Config::default()).await?;
        f.service.execute(f.id).await?;
        let paused = tokio::time::timeout(Duration::from_secs(60), async {
            let mut changed = f.service.store().subscribe();
            loop {
                let run = f.service.store().engineering_get(f.id).await?;
                if run.tasks[0].status == TaskStatus::BlockedHuman
                    && f.root.join("c-started").exists()
                {
                    break Ok::<_, anyhow::Error>(run);
                }
                changed.changed().await?;
            }
        })
        .await??;
        let journal = f
            .service
            .registry()
            .journal
            .clone()
            .expect("active journal");
        let gate = journal.pause_resolution_publication().await;
        struct Release(Arc<super::super::journal::PublicationGate>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.publish.notify_one();
            }
        }
        let _release = Release(gate.clone());
        let request = ResolutionRequest {
            request_id: Uuid::new_v4(),
            disposition: ResolutionDisposition::Resolve,
            note: "Use the selected behavior; inspected all effects".into(),
            effect: EffectState::NotPerformed,
            permission: None,
        };
        let action_id = paused.human_actions[0].id.clone();
        let caller = {
            let service = f.service.clone();
            let request = request.clone();
            let action_id = action_id.clone();
            tokio::spawn(async move { service.resolve(f.id, &action_id, request).await })
        };
        tokio::time::timeout(Duration::from_secs(10), gate.committed.notified()).await?;
        let committed = f.service.store().engineering_get(f.id).await?;
        assert_eq!(
            committed.human_actions[0].status,
            HumanActionStatus::Resolved
        );
        caller.abort();
        assert!(
            caller
                .await
                .expect_err("request should be cancelled")
                .is_cancelled()
        );
        let duplicate = {
            let service = f.service.clone();
            let request = request.clone();
            let action_id = action_id.clone();
            tokio::spawn(async move { service.resolve(f.id, &action_id, request).await })
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while f.service.inner.mutations.available_permits()
                != f.config.runner.max_pending_submissions - 2
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        let shutdown = {
            let service = f.service.clone();
            tokio::spawn(async move { service.shutdown().await })
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while f.service.registry().accepting {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        assert!(
            !shutdown.is_finished(),
            "shutdown abandoned the accepted mutation"
        );
        gate.publish.notify_one();
        let retried = duplicate.await??;
        assert_eq!(
            serde_json::to_value(&retried)?,
            serde_json::to_value(&committed)?,
            "retry must return the original result"
        );
        shutdown.await??;
        let durable = f.service.store().engineering_get(f.id).await?;
        assert_eq!(durable.status, EngineeringStatus::Interrupted);
        assert!(
            durable
                .tasks
                .iter()
                .all(|t| t.status != TaskStatus::Running)
        );
        assert_eq!(f.service.health(), (None, None));
        assert_eq!(
            serde_json::to_value(journal.read().await)?,
            serde_json::to_value(&durable)?
        );
        let retry = f.service.resolve(f.id, &action_id, request.clone()).await?;
        assert_eq!(
            serde_json::to_value(retry)?,
            serde_json::to_value(&committed)?
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM engineering_resolution_requests WHERE request_id=?",
        )
        .bind(request.request_id.to_string())
        .fetch_one(&f.service.store().readers)
        .await?;
        assert_eq!(count, 1);
        assert_eq!(durable.human_actions[0].audit.len(), 2);
        f.supervisor.shutdown().await
    }

    #[tokio::test]
    async fn production_preflight_uses_fake_claude_and_codex_help_only() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir()?;
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin)?;
        let script = r#"#!/bin/sh
printf '%s %s\n' "$0" "$*" >> "$AUTONO_FAKE_PROBE_LOG"
case "$*" in
  '--help')
    case "$0" in
      */codex) printf '%s\n' '--no-daemon' ;;
      */claude)
        if [ "$AUTONO_BAD_CLAUDE_HELP" = 1 ]; then printf '%s\n' '--print stream-json --json-schema --permission-prompts --tools --strict-mcp-config'
        else printf '%s\n' '--print stream-json --json-schema --permission-prompts --safe-mode --tools --strict-mcp-config'; fi ;;
    esac ;;
  'exec --help') printf '%s\n' '--json --sandbox --output-schema --ignore-user-config' ;;
  *) printf '%s\n' 'UNEXPECTED INFERENCE ATTEMPT' >> "$AUTONO_FAKE_PROBE_LOG"; exit 91 ;;
esac
"#;
        for name in ["codex", "claude"] {
            let path = bin.join(name);
            std::fs::write(&path, script)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        for incompatible in [false, true] {
            let output = tokio::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "engineering::service::tests::production_preflight_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("PATH", std::env::join_paths(&paths)?)
                .env("AUTONO_FAKE_PROBE_ROOT", root.path())
                .env("AUTONO_FAKE_PROBE_LOG", root.path().join("probe.log"))
                .env(
                    "AUTONO_PROBE_SECRET",
                    "synthetic-preflight-value-never-persist",
                )
                .env(
                    "AUTONO_BAD_CLAUDE_HELP",
                    if incompatible { "1" } else { "0" },
                )
                .output()
                .await?;
            ensure!(
                output.status.success(),
                "fake production preflight child failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let log = std::fs::read_to_string(root.path().join("probe.log"))?;
        assert!(log.lines().all(|line| line.ends_with(" --help")));
        assert_eq!(
            log.lines().count(),
            12,
            "both provider/role combinations must use help probes only"
        );
        Ok(())
    }
    #[tokio::test]
    #[ignore = "isolated subprocess helper; parent supplies fake provider executables"]
    async fn production_preflight_child() -> Result<()> {
        ensure!(
            std::env::var_os("AUTONO_FAKE_PROBE_ROOT").is_some(),
            "refuse to probe real provider executables"
        );
        for reverse in [false, true] {
            let mut config = Config::default();
            config.preflight.required_env = vec!["AUTONO_PROBE_SECRET".into()];
            if reverse {
                config.agents.coder.provider = Provider::Claude;
                config.agents.reviewer.provider = Provider::Codex;
            }
            let mut f = Fixture::planned(config).await?;
            f.service.shutdown().await?;
            f.service = EngineeringService::new(
                f.supervisor.clone(),
                f.root.join("home"),
                f.config.clone(),
            );
            let before = f.service.store().engineering_get(f.id).await?;
            let artifacts = f
                .root
                .join("home/engineering")
                .join(f.id.to_string())
                .join("artifacts");
            let sessions = std::fs::read_dir(&artifacts)?.count();
            let run = f.service.preflight(f.id).await?;
            let report = run.preflight.expect("report");
            let compatible = std::env::var("AUTONO_BAD_CLAUDE_HELP")? == "0";
            assert_eq!(report.ready, compatible, "{report:?}");
            for (role, provider) in [
                ("coder", f.config.agents.coder.provider),
                ("reviewer", f.config.agents.reviewer.provider),
            ] {
                assert_eq!(
                    report
                        .checks
                        .iter()
                        .find(|c| c.id == format!("provider-{role}"))
                        .expect("provider check")
                        .ready,
                    Some(compatible || provider == Provider::Codex)
                );
            }
            assert!(
                !before
                    .planning_workspace
                    .path
                    .join("unused-preflight-schema")
                    .exists()
            );
            assert_eq!(
                std::fs::read_dir(artifacts)?.count(),
                sessions,
                "readiness created invocation artifacts"
            );
            assert!(
                !serde_json::to_string(&report)?
                    .contains("synthetic-preflight-value-never-persist")
            );
            f.service.shutdown().await?;
            f.supervisor.shutdown().await?;
        }
        Ok(())
    }
}
