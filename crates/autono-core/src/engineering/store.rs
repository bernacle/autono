use super::*;
use crate::{Run, RunStatus, Store};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
#[error("engineering run {0} not found")]
pub struct EngineeringNotFound(pub Uuid);
#[derive(Debug, thiserror::Error)]
#[error("engineering state conflict: {0}")]
pub struct EngineeringConflict(pub String);

impl Store {
    pub async fn engineering_get(&self, id: Uuid) -> Result<EngineeringRun> {
        let doc: Option<String> =
            sqlx::query_scalar("SELECT document FROM engineering_runs WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&self.readers)
                .await?;
        serde_json::from_str(&doc.ok_or(EngineeringNotFound(id))?).context("decode engineering run")
    }
    pub async fn engineering_list(&self, limit: u32, offset: u32) -> Result<Vec<EngineeringRun>> {
        let docs: Vec<String> = sqlx::query_scalar(
            "SELECT document FROM engineering_runs ORDER BY rowid DESC LIMIT ? OFFSET ?",
        )
        .bind(limit.min(100))
        .bind(offset)
        .fetch_all(&self.readers)
        .await?;
        docs.iter()
            .map(|d| serde_json::from_str(d).context("decode engineering run"))
            .collect()
    }
    pub(crate) async fn engineering_create(&self, run: &EngineeringRun) -> Result<()> {
        run.settings.validate()?;
        ensure!(
            run.status == EngineeringStatus::Planning
                && run.revision == 0
                && run.plan.is_none()
                && run.tasks.is_empty(),
            "invalid initial engineering run"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO engineering_runs(id,revision,status,document) VALUES(?,0,'PLANNING',?)",
        )
        .bind(run.id.to_string())
        .bind(serde_json::to_string(run)?)
        .execute(&mut *tx)
        .await?;
        reserve(&mut tx, run, &run.planning_workspace).await?;
        append(
            &mut tx,
            run.id,
            "created",
            "Feature specification and configuration preserved; planning requested",
        )
        .await?;
        tx.commit().await?;
        self.changed.send_replace(());
        Ok(())
    }
    /// Optimistic revision and evidence checks share the state/event transaction.
    pub(crate) async fn engineering_save(
        &self,
        run: &EngineeringRun,
        kind: &str,
        detail: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let doc: String = sqlx::query_scalar("SELECT document FROM engineering_runs WHERE id=?")
            .bind(run.id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let old: EngineeringRun = serde_json::from_str(&doc)?;
        ensure!(
            old.revision + 1 == run.revision,
            EngineeringConflict("stale revision".into())
        );
        ensure!(
            old.status.can_transition(run.status),
            EngineeringConflict(format!(
                "illegal run transition {:?} -> {:?}",
                old.status, run.status
            ))
        );
        ensure!(
            old.specification == run.specification
                && old.repository == run.repository
                && old.repository_git == run.repository_git
                && old.requested_base == run.requested_base
                && old.base_commit == run.base_commit
                && old.settings == run.settings,
            "immutable run inputs changed"
        );
        if old.plan.is_some() {
            ensure!(old.plan == run.plan, "approved plan cannot be rewritten");
        }
        if let Some(plan) = &run.plan {
            plan.validate()?;
            ensure!(
                run.tasks.iter().map(|t| &t.plan).eq(plan.tasks.iter()),
                "task definitions differ from plan"
            );
        }
        for task in &run.tasks {
            if let Some(previous) = old.tasks.iter().find(|t| t.plan.id == task.plan.id) {
                ensure!(
                    previous.status.can_transition(task.status),
                    "illegal task transition"
                );
            } else {
                ensure!(
                    old.status == EngineeringStatus::Planning,
                    "cannot add tasks after planning"
                );
            }
            for attempt in &task.attempts {
                for review in &attempt.reviews {
                    if let Some(workspace) = &review.workspace {
                        reserve(&mut tx, run, workspace).await?;
                    }
                }
            }
            if let Some(workspace) = &task.workspace {
                reserve(&mut tx, run, workspace).await?;
            }
            if task.status == TaskStatus::Completed {
                ensure!(
                    task.artifact_commit.is_some()
                        && task
                            .workspace
                            .as_ref()
                            .is_some_and(|w| w.head_commit == task.artifact_commit),
                    "completed task has no preserved artifact"
                );
                let last = task
                    .attempts
                    .last()
                    .context("completed task has no attempt")?;
                let review = last
                    .review
                    .as_ref()
                    .context("completed task has no review")?;
                review.validate(&task.plan)?;
                ensure!(
                    review.accepts() && last.snapshot.is_some(),
                    "completed task lacks acceptance evidence"
                );
                let reviewer = last
                    .reviewer
                    .as_ref()
                    .context("missing independent review session")?;
                ensure!(
                    reviewer.id != last.coder.id && reviewer.resumed_from.is_none(),
                    "reviewer must be a fresh independent session"
                );
                for session in [&last.coder, reviewer] {
                    verify_process(
                        &mut tx,
                        session.process_run_id.context("missing agent process")?,
                        None,
                    )
                    .await?;
                    ensure!(
                        session.outcome.as_deref() == Some("succeeded"),
                        "agent session did not succeed"
                    );
                }
                verify_checks(&mut tx, &run.settings, &last.verification).await?;
            }
        }
        if let Some(workspace) = &run.integration_workspace {
            reserve(&mut tx, run, workspace).await?;
        }
        if matches!(
            run.status,
            EngineeringStatus::Running
                | EngineeringStatus::Verified
                | EngineeringStatus::Integrating
                | EngineeringStatus::Completed
        ) {
            ensure!(
                run.approved_at.is_some() && run.plan.is_some(),
                "execution requires explicit plan approval"
            );
        }
        if matches!(
            run.status,
            EngineeringStatus::Verified
                | EngineeringStatus::Integrating
                | EngineeringStatus::Completed
        ) {
            ensure!(
                run.tasks.iter().all(|t| t.status == TaskStatus::Completed),
                "run contains incomplete tasks"
            );
        }
        if run.status == EngineeringStatus::Completed {
            ensure!(
                run.integration_workspace
                    .as_ref()
                    .is_some_and(|w| w.head_commit.is_some()),
                "integration artifact missing"
            );
            verify_checks(&mut tx, &run.settings, &run.integration_verification).await?;
        }
        sqlx::query(
            "UPDATE engineering_runs SET revision=?,status=?,document=? WHERE id=? AND revision=?",
        )
        .bind(run.revision)
        .bind(
            serde_json::to_value(run.status)?
                .as_str()
                .context("status")?,
        )
        .bind(serde_json::to_string(run)?)
        .bind(run.id.to_string())
        .bind(old.revision)
        .execute(&mut *tx)
        .await?;
        append(&mut tx, run.id, kind, detail).await?;
        for task in &run.tasks {
            let from = old
                .tasks
                .iter()
                .find(|t| t.plan.id == task.plan.id)
                .map(|t| t.status);
            if from != Some(task.status) {
                append(
                    &mut tx,
                    run.id,
                    "task_state",
                    &format!("{}: {:?} -> {:?}", task.plan.id, from, task.status),
                )
                .await?;
            }
        }
        tx.commit().await?;
        self.changed.send_replace(());
        Ok(())
    }
    pub async fn engineering_events(
        &self,
        id: Uuid,
        after: i64,
        limit: u32,
    ) -> Result<Vec<EngineeringEvent>> {
        let rows = sqlx::query("SELECT sequence,timestamp,kind,detail FROM engineering_events WHERE run_id=? AND sequence>? ORDER BY sequence LIMIT ?").bind(id.to_string()).bind(after).bind(limit.min(500)).fetch_all(&self.readers).await?;
        rows.iter()
            .map(|r| {
                Ok(EngineeringEvent {
                    sequence: r.try_get("sequence")?,
                    run_id: id,
                    timestamp: r.try_get::<String, _>("timestamp")?.parse()?,
                    kind: r.try_get("kind")?,
                    detail: r.try_get("detail")?,
                })
            })
            .collect()
    }
    /// No automatic resumption: a crashed agent may still be writing. Never adopt saved PIDs.
    pub async fn engineering_reconcile(&self) -> Result<usize> {
        let docs: Vec<String> = sqlx::query_scalar("SELECT document FROM engineering_runs WHERE status IN ('PLANNING','RUNNING','INTEGRATING')").fetch_all(&self.readers).await?;
        for doc in &docs {
            let mut run: EngineeringRun = serde_json::from_str(doc)?;
            run.status = EngineeringStatus::Interrupted;
            run.revision += 1;
            run.updated_at = Utc::now();
            for task in &mut run.tasks {
                if matches!(
                    task.status,
                    TaskStatus::Waiting | TaskStatus::Ready | TaskStatus::Running
                ) {
                    task.status = TaskStatus::Interrupted;
                }
            }
            run.summary = "Daemon restarted; workflow ownership lost. Workspaces retained. Inspect surviving processes before starting a new run.".into();
            run.blockers.push(Blocker {
                task_id: None,
                kind: "restart".into(),
                detail: run.summary.clone(),
                action: "Inspect retained workspaces and processes; submit a new plan when safe"
                    .into(),
                created_at: Utc::now(),
            });
            self.engineering_save(&run, "interrupted", &run.summary)
                .await?;
        }
        Ok(docs.len())
    }
}
async fn reserve(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    run: &EngineeringRun,
    w: &Workspace,
) -> Result<()> {
    sqlx::query("INSERT INTO engineering_workspaces(id,run_id,path,branch,repository) VALUES(?,?,?,?,?) ON CONFLICT(id) DO NOTHING").bind(w.id.to_string()).bind(run.id.to_string()).bind(w.path.to_string_lossy().as_ref()).bind(&w.branch).bind(run.repository.to_string_lossy().as_ref()).execute(&mut **tx).await?;
    let row =
        sqlx::query("SELECT run_id,path,branch,repository FROM engineering_workspaces WHERE id=?")
            .bind(w.id.to_string())
            .fetch_one(&mut **tx)
            .await?;
    ensure!(
        row.try_get::<String, _>("run_id")? == run.id.to_string()
            && row.try_get::<String, _>("path")? == w.path.to_string_lossy()
            && row.try_get::<String, _>("branch")? == w.branch
            && row.try_get::<String, _>("repository")? == run.repository.to_string_lossy(),
        "workspace ownership conflict"
    );
    Ok(())
}
async fn verify_process(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: Uuid,
    command: Option<&Vec<String>>,
) -> Result<()> {
    let doc: String = sqlx::query_scalar("SELECT document FROM runs WHERE id=?")
        .bind(id.to_string())
        .fetch_one(&mut **tx)
        .await?;
    let run: Run = serde_json::from_str(&doc)?;
    ensure!(
        run.status == RunStatus::Succeeded
            && run.exit_code == Some(0)
            && run.output_complete == Some(true),
        "process evidence is incomplete or unsuccessful"
    );
    if let Some(command) = command {
        ensure!(
            &run.spec.command == command,
            "verification command does not match evidence"
        );
    }
    Ok(())
}
async fn verify_checks(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    settings: &WorkflowSettings,
    checks: &[VerificationResult],
) -> Result<()> {
    ensure!(
        checks.len() == settings.verification.commands.len(),
        "missing configured verification checks"
    );
    for (check, command) in checks.iter().zip(&settings.verification.commands) {
        ensure!(
            check.passed
                && check.exit_code == Some(0)
                && check.output_complete
                && &check.command == command,
            "required verification failed"
        );
        verify_process(tx, check.process_run_id, Some(command)).await?;
    }
    Ok(())
}
async fn append(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: Uuid,
    kind: &str,
    detail: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO engineering_events(run_id,timestamp,kind,detail) VALUES(?,?,?,?)")
        .bind(id.to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(kind)
        .bind(detail)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RunSpec, engineering::journal::Journal};
    fn fixture(root: &std::path::Path) -> EngineeringRun {
        let id = Uuid::new_v4();
        let now = Utc::now();
        EngineeringRun {
            id,
            revision: 0,
            status: EngineeringStatus::Planning,
            created_at: now,
            updated_at: now,
            repository: root.to_owned(),
            repository_git: None,
            requested_base: "HEAD".into(),
            base_commit: "a".repeat(40),
            specification: "A feature".into(),
            settings: WorkflowSettings::default(),
            plan: None,
            approved_at: None,
            planner: None,
            planning_workspace: super::super::workspace::reference(
                root,
                id,
                "planner",
                &"a".repeat(40),
            ),
            tasks: vec![],
            blockers: vec![],
            integration_workspace: None,
            integration_verification: vec![],
            summary: "Planning".into(),
        }
    }
    fn plan() -> ExecutionPlan {
        ExecutionPlan {
            requirements: vec![Requirement {
                id: "r".into(),
                text: "Original requirement".into(),
            }],
            milestones: vec![Milestone {
                id: "m".into(),
                goal: "Milestone".into(),
                task_ids: vec!["t".into()],
            }],
            tasks: vec![PlannedTask {
                id: "t".into(),
                goal: "Goal".into(),
                description: "Description".into(),
                acceptance_criteria: vec!["Criterion".into()],
                dependencies: vec![],
                requirement_ids: vec!["r".into()],
                verification_expectations: vec!["Checks pass".into()],
                role: AgentRole::Coder,
            }],
        }
    }
    async fn approved(store: &Store, root: &std::path::Path) -> Result<Journal> {
        let run = fixture(root);
        store.engineering_create(&run).await?;
        let journal = Journal::new(store.clone(), run);
        journal
            .change("plan", "Plan validated", |r| {
                let p = plan();
                r.tasks = p
                    .tasks
                    .iter()
                    .map(|p| EngineeringTask {
                        plan: p.clone(),
                        status: TaskStatus::Ready,
                        workspace: None,
                        attempts: vec![],
                        artifact_commit: None,
                    })
                    .collect();
                r.plan = Some(p);
                r.status = EngineeringStatus::AwaitingApproval;
                Ok(())
            })
            .await?;
        journal
            .change("approved", "Developer approved", |r| {
                r.approved_at = Some(Utc::now());
                r.status = EngineeringStatus::Running;
                r.tasks[0].status = TaskStatus::Running;
                Ok(())
            })
            .await?;
        Ok(journal)
    }
    #[tokio::test]
    async fn revisions_append_only_events_and_restart_reconciliation_are_durable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("db");
        let store = Store::open(&path).await?;
        let journal = approved(&store, dir.path()).await?;
        let run = journal.read().await;
        assert!(
            store
                .engineering_save(&run, "stale", "must not commit")
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM engineering_events")
                .execute(&store.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("UPDATE engineering_events SET detail='tampered'")
                .execute(&store.pool)
                .await
                .is_err()
        );
        drop(journal);
        drop(store);
        let reopened = Store::open(&path).await?;
        assert_eq!(reopened.engineering_reconcile().await?, 1);
        let run = reopened.engineering_get(run.id).await?;
        assert_eq!(run.status, EngineeringStatus::Interrupted);
        assert_eq!(run.tasks[0].status, TaskStatus::Interrupted);
        assert!(!run.blockers.is_empty());
        assert!(run.planning_workspace.path.parent().is_some());
        assert_eq!(reopened.engineering_reconcile().await?, 0);
        assert!(
            reopened
                .engineering_events(run.id, 0, 100)
                .await?
                .iter()
                .any(|e| e.kind == "interrupted")
        );
        Ok(())
    }
    #[tokio::test]
    async fn forged_completion_rolls_back_without_verified_process_evidence() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("db")).await?;
        let journal = approved(&store, dir.path()).await?;
        let mut run = journal.read().await;
        let count = store.engineering_events(run.id, 0, 100).await?.len();
        let process = store
            .create(RunSpec {
                command: vec!["false".into()],
                cwd: dir.path().to_owned(),
                env: Default::default(),
                timeout_seconds: Some(1),
                keep_awake: false,
            })
            .await?;
        store
            .transition(
                process.id,
                RunStatus::Failed,
                None,
                Some(42),
                Some("failed".into()),
            )
            .await?;
        let session = |role| AgentSession {
            id: Uuid::new_v4(),
            role,
            provider: Provider::Codex,
            provider_session_id: Some(Uuid::new_v4().to_string()),
            resumed_from: None,
            process_key: Uuid::new_v4(),
            process_run_id: Some(process.id),
            outcome: Some("succeeded".into()),
            result: Some("forged success".into()),
        };
        let task = &mut run.tasks[0];
        let mut w =
            super::super::workspace::reference(dir.path(), run.id, "task-t", &run.base_commit);
        w.head_commit = Some("b".repeat(40));
        task.workspace = Some(w);
        task.status = TaskStatus::Completed;
        task.artifact_commit = Some("b".repeat(40));
        task.attempts.push(TaskAttempt {
            number: 1,
            coder: session(AgentRole::Coder),
            verification: run
                .settings
                .verification
                .commands
                .iter()
                .map(|command| VerificationResult {
                    process_key: Uuid::new_v4(),
                    process_run_id: process.id,
                    command: command.clone(),
                    exit_code: Some(0),
                    passed: true,
                    outcome: Some(VerificationOutcome::Passed),
                    output_complete: true,
                    summary: "forged check".into(),
                })
                .collect(),
            reviews: vec![],
            reviewer: Some(session(AgentRole::Reviewer)),
            review: Some(ReviewResult {
                accepted: true,
                criteria: vec![CriterionReview {
                    criterion_id: String::new(),
                    criterion: "Criterion".into(),
                    satisfied: true,
                    evidence: "Claimed evidence".into(),
                }],
                findings: vec![],
                blocker: String::new(),
            }),
            snapshot: Some("snapshot".into()),
            detail: None,
        });
        run.revision += 1;
        assert!(
            store
                .engineering_save(&run, "completed", "must reject forged evidence")
                .await
                .is_err()
        );
        assert_eq!(
            store.engineering_get(run.id).await?.tasks[0].status,
            TaskStatus::Running
        );
        assert_eq!(store.engineering_events(run.id, 0, 100).await?.len(), count);
        Ok(())
    }
}
