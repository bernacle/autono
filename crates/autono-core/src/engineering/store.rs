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
    pub async fn engineering_delegation_proposal(&self, id: Uuid) -> Result<DelegationContract> {
        let document: String = sqlx::query_scalar(
            "SELECT document FROM engineering_delegation_proposals WHERE run_id=?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.readers)
        .await?
        .ok_or(EngineeringNotFound(id))?;
        Ok(serde_json::from_str(&document)?)
    }
    pub(super) async fn engineering_propose(&self, contract: &DelegationContract) -> Result<()> {
        sqlx::query("INSERT INTO engineering_delegation_proposals(run_id,document) VALUES(?,?) ON CONFLICT DO NOTHING")
            .bind(contract.request.submission.id.to_string()).bind(serde_json::to_string(contract)?).execute(&self.pool).await?;
        ensure!(
            self.engineering_delegation_proposal(contract.request.submission.id)
                .await?
                == *contract,
            EngineeringConflict("delegation ID already has a different proposal".into())
        );
        Ok(())
    }
    pub async fn engineering_get(&self, id: Uuid) -> Result<EngineeringRun> {
        let doc: Option<String> =
            sqlx::query_scalar("SELECT document FROM engineering_runs WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&self.readers)
                .await?;
        serde_json::from_str(&doc.ok_or(EngineeringNotFound(id))?).context("decode engineering run")
    }
    pub(crate) async fn engineering_resolution_result(
        &self,
        id: Uuid,
        action: &str,
        request: &ResolutionRequest,
    ) -> Result<Option<EngineeringRun>> {
        let row = sqlx::query("SELECT run_id,action_id,document,result FROM engineering_resolution_requests WHERE request_id=?").bind(request.request_id.to_string()).fetch_optional(&self.readers).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        ensure!(
            row.try_get::<String, _>("run_id")? == id.to_string()
                && row.try_get::<String, _>("action_id")? == action
                && serde_json::from_str::<ResolutionRequest>(
                    &row.try_get::<String, _>("document")?
                )? == *request,
            EngineeringConflict("resolution key has another payload, action or run".into())
        );
        Ok(Some(match row.try_get::<Option<String>, _>("result")? {
            Some(result) => serde_json::from_str(&result)?,
            // Older receipts predate response snapshots. They remain idempotent;
            // only their current run view is available, never a fabricated history.
            None => self.engineering_get(id).await?,
        }))
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
                && run.tasks.is_empty()
                && run.decisions.is_empty(),
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
        if let Some(auth) = &run.delegation {
            ensure!(
                auth.contract.request.submission.id == run.id
                    && auth.expires_at
                        == auth.authorized_at
                            + chrono::Duration::seconds(
                                auth.contract.request.max_runtime_seconds as i64
                            ),
                "invalid delegation lifetime or run binding"
            );
            sqlx::query("INSERT INTO engineering_delegations(run_id,document) VALUES(?,?)")
                .bind(run.id.to_string())
                .bind(serde_json::to_string(auth)?)
                .execute(&mut *tx)
                .await?;
            append(&mut tx, run.id, "delegation_authorized", "Developer consented to the exact immutable bounded contract; no monetary accounting is available").await?;
        }
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
                && old.settings == run.settings
                && old.delegation == run.delegation,
            "immutable run inputs changed"
        );
        save_decisions(&mut tx, &old, run).await?;
        if old.status != EngineeringStatus::Running && run.status == EngineeringStatus::Running {
            ensure!(
                run.preflight
                    .as_ref()
                    .is_some_and(|p| (p.ready || run.autonomous_preflight_ready())
                        && !p.checks.iter().any(|c| c.blocking && c.ready != Some(true))),
                "execution requires a ready preflight report"
            );
        }
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
                if previous.status == TaskStatus::BlockedHuman && task.status == TaskStatus::Ready {
                    let actions: Vec<_> = run
                        .human_actions
                        .iter()
                        .filter(|a| {
                            a.requirement.kind == HumanActionType::Runtime
                                && a.requirement.affected_tasks.contains(&task.plan.id)
                        })
                        .collect();
                    ensure!(
                        !actions.is_empty()
                            && actions.iter().all(|a| a.continuable
                                && a.status == HumanActionStatus::Resolved
                                && a.resolution
                                    .as_ref()
                                    .is_some_and(|r| r.effect != EffectState::Unknown)),
                        "human pause requires explicit resolved action and inspected effects"
                    );
                }
                if previous.status == TaskStatus::Completed {
                    ensure!(
                        serde_json::to_value(previous)? == serde_json::to_value(task)?,
                        "completed task evidence is immutable"
                    );
                }
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
                if let Some(ids) = &last.decision_ids {
                    review.validate_decisions(ids)?;
                }
                ensure!(
                    review.accepts() && last.snapshot.is_some(),
                    "completed task lacks acceptance evidence"
                );
                // Existing completed records remain readable. Every new completion
                // must carry matching immutable verified/preserved tree evidence.
                let legacy_completed = old.tasks.iter().any(|t| {
                    t.plan.id == task.plan.id
                        && t.status == TaskStatus::Completed
                        && t.attempts.last().is_some_and(|a| {
                            a.verified_tree.is_none() && a.preserved_tree.is_none()
                        })
                }) && last.verified_tree.is_none()
                    && last.preserved_tree.is_none();
                ensure!(
                    legacy_completed
                        || (last.verified_tree.is_some()
                            && last.verified_tree == last.preserved_tree),
                    "completed task lacks matching verified/preserved trees"
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
                        None,
                    )
                    .await?;
                    ensure!(
                        session.outcome.as_deref() == Some("succeeded"),
                        "agent session did not succeed"
                    );
                }
                verify_checks(
                    &mut tx,
                    run,
                    task.workspace
                        .as_ref()
                        .context("task workspace missing")?
                        .path
                        .as_path(),
                    &last.verification,
                )
                .await?;
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
            verify_checks(
                &mut tx,
                run,
                run.integration_workspace
                    .as_ref()
                    .context("integration workspace missing")?
                    .path
                    .as_path(),
                &run.integration_verification,
            )
            .await?;
        }
        // Human checklist projections and idempotency receipts commit with the run/event.
        ensure!(
            old.human_actions
                .iter()
                .all(|a| run.human_actions.iter().any(|b| b.id == a.id
                    && b.requirement == a.requirement
                    && b.created_at == a.created_at
                    && b.continuable == a.continuable
                    && b.workspace_snapshot == a.workspace_snapshot
                    && b.audit.starts_with(&a.audit))),
            "human action identity/history is immutable"
        );
        for action in &run.human_actions {
            sqlx::query("INSERT INTO engineering_human_actions(run_id,action_id,status,document) VALUES(?,?,?,?) ON CONFLICT(run_id,action_id) DO UPDATE SET status=excluded.status,document=excluded.document")
                .bind(run.id.to_string()).bind(&action.id).bind(serde_json::to_value(action.status)?.as_str().context("action status")?).bind(serde_json::to_string(action)?).execute(&mut *tx).await?;
            for request in action.audit.iter().filter_map(|a| a.request.as_ref()) {
                let payload = serde_json::to_string(request)?;
                sqlx::query("INSERT INTO engineering_resolution_requests(request_id,run_id,action_id,document,result) VALUES(?,?,?,?,?) ON CONFLICT(request_id) DO NOTHING")
                    .bind(request.request_id.to_string()).bind(run.id.to_string()).bind(&action.id).bind(&payload).bind(serde_json::to_string(run)?).execute(&mut *tx).await?;
                let row = sqlx::query("SELECT run_id,action_id,document FROM engineering_resolution_requests WHERE request_id=?").bind(request.request_id.to_string()).fetch_one(&mut *tx).await?;
                ensure!(
                    row.try_get::<String, _>("run_id")? == run.id.to_string()
                        && row.try_get::<String, _>("action_id")? == action.id
                        && row.try_get::<String, _>("document")? == payload,
                    EngineeringConflict("resolution request ID has another payload".into())
                );
            }
        }
        if let Some(report) = &run.preflight {
            sqlx::query("INSERT INTO engineering_preflight(run_id,revision,document) VALUES(?,?,?) ON CONFLICT(run_id) DO UPDATE SET revision=excluded.revision,document=excluded.document")
                .bind(run.id.to_string()).bind(run.revision).bind(serde_json::to_string(report)?).execute(&mut *tx).await?;
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
                    TaskStatus::Waiting
                        | TaskStatus::Ready
                        | TaskStatus::Running
                        | TaskStatus::BlockedHuman
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
    scope: Option<(&std::path::Path, u64, Uuid)>,
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
    if let Some((cwd, seconds, key)) = scope {
        ensure!(
            run.spec.cwd == cwd
                && run.spec.timeout_seconds == Some(seconds)
                && run.submission_key == Some(key),
            "delegated verification process context does not match evidence"
        );
    }
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
    run: &EngineeringRun,
    cwd: &std::path::Path,
    checks: &[VerificationResult],
) -> Result<()> {
    ensure!(
        checks.len() == run.settings.verification.commands.len(),
        "missing configured verification checks"
    );
    for (check, command) in checks.iter().zip(&run.settings.verification.commands) {
        ensure!(
            check.passed
                && check.exit_code == Some(0)
                && check.output_complete
                && &check.command == command,
            "required verification failed"
        );
        let effective = if let Some(auth) = &run.delegation {
            super::delegation::sandbox_command(
                command,
                cwd,
                &auth
                    .contract
                    .worktree_root
                    .join("check-tmp")
                    .join(check.process_key.to_string()),
            )?
        } else {
            command.clone()
        };
        verify_process(
            tx,
            check.process_run_id,
            Some(&effective),
            run.delegation.as_ref().map(|_| {
                (
                    cwd,
                    run.settings.verification.timeout_seconds,
                    check.process_key,
                )
            }),
        )
        .await?;
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

/// Decision identity and all prior events are immutable, in the run revision transaction.
async fn save_decisions(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    old: &EngineeringRun,
    run: &EngineeringRun,
) -> Result<()> {
    // Attempt bindings are part of the same transaction as immutable ledger
    // events. Once admitted, neither context nor consumed revisions may change.
    for task in &run.tasks {
        let previous = old.tasks.iter().find(|t| t.plan.id == task.plan.id);
        ensure!(
            previous.is_none_or(|t| task.attempts.len() >= t.attempts.len()),
            "attempt history removed"
        );
        for (index, attempt) in task.attempts.iter().enumerate() {
            ensure!(attempt.number == index + 1, "invalid attempt order");
            let prior = previous.and_then(|t| t.attempts.get(index));
            if let Some(prior) = prior {
                ensure!(
                    prior.decision_context == attempt.decision_context
                        && prior
                            .decision_ids
                            .as_ref()
                            .is_none_or(|ids| Some(ids) == attempt.decision_ids.as_ref()),
                    "attempt decision binding is immutable"
                );
            } else {
                ensure!(
                    attempt.decision_context == super::decisions::active_ids(old, &task.plan.id),
                    "attempt context differs from admitted decision revisions"
                );
            }
            if let Some(ids) = &attempt.decision_ids {
                ensure!(
                    run.delegation.is_some(),
                    "supervised attempts cannot consume autonomous decisions"
                );
                if prior.is_none_or(|p| p.decision_ids.is_none()) {
                    ensure!(
                        run.decisions
                            .iter()
                            .filter(|d| ids.contains(&d.id))
                            .all(|d| super::decisions::trusted_basis(run, d)),
                        "attempt cannot consume historical unverified provenance"
                    );
                    ensure!(
                        *ids == super::decisions::active_ids(run, &task.plan.id),
                        "attempt did not consume active revisions"
                    );
                    ensure!(
                        attempt.decision_context.iter().all(|id| ids.contains(id)
                            || run.decisions.iter().any(|d| d.proposal.supersedes.as_ref()
                                == Some(id)
                                && d.task_id.as_ref() == Some(&task.plan.id)
                                && d.attempt == Some(attempt.number)
                                && ids.contains(&d.id)))
                            && ids.iter().all(|id| attempt.decision_context.contains(id)
                                || run.decisions.iter().any(|d| &d.id == id
                                    && d.task_id.as_ref() == Some(&task.plan.id)
                                    && d.attempt == Some(attempt.number))),
                        "attempt consumed revisions outside its context and own proposals"
                    );
                    ensure!(
                        ids.iter().all(|id| run.decisions.iter().any(|d| &d.id == id
                            && d.audit.iter().any(|e| e.kind == DecisionEventKind::Consumed
                                && e.task_id.as_ref() == Some(&task.plan.id)
                                && e.attempt == Some(attempt.number)))),
                        "attempt binding lacks immutable consumption evidence"
                    );
                }
            }
        }
    }
    ensure!(
        run.decisions.len() >= old.decisions.len() && run.decisions.len() <= 128,
        "decision history removed or oversized"
    );
    for (i, d) in run.decisions.iter().enumerate() {
        ensure!(
            run.delegation.is_some()
                && d.run_id == run.id
                && d.ordinal == i + 1
                && !d.audit.is_empty(),
            "invalid decision binding"
        );
        let mut identity = d.clone();
        identity.audit.clear();
        if let Some(prior) = old.decisions.get(i) {
            let mut previous = prior.clone();
            previous.audit.clear();
            ensure!(
                identity == previous && d.audit.starts_with(&prior.audit),
                "decision history is immutable"
            );
        } else {
            sqlx::query(
                "INSERT INTO engineering_decisions(run_id,id,ordinal,document) VALUES(?,?,?,?)",
            )
            .bind(run.id.to_string())
            .bind(&d.id)
            .bind(d.ordinal as i64)
            .bind(serde_json::to_string(&identity)?)
            .execute(&mut **tx)
            .await?;
        }
        let prior_len = old.decisions.get(i).map_or(0, |d| d.audit.len());
        for (j, event) in d.audit.iter().enumerate().skip(prior_len) {
            ensure!(event.sequence == j + 1, "invalid decision event order");
            if matches!(
                event.kind,
                DecisionEventKind::Consumed
                    | DecisionEventKind::Implemented
                    | DecisionEventKind::VerificationFailed
                    | DecisionEventKind::ChecksPassed
                    | DecisionEventKind::ReviewAccepted
                    | DecisionEventKind::ReviewRejected
                    | DecisionEventKind::TaskVerified
            ) {
                let task = run
                    .tasks
                    .iter()
                    .find(|t| Some(&t.plan.id) == event.task_id.as_ref())
                    .context("decision event lacks task")?;
                let a = task
                    .attempts
                    .iter()
                    .find(|a| Some(a.number) == event.attempt)
                    .context("decision event lacks attempt")?;
                ensure!(
                    d.autonomous()
                        && a.decision_ids
                            .as_ref()
                            .is_some_and(|ids| ids.contains(&d.id)),
                    "decision event does not match the consumed revision"
                );
                let prior_event = |kind| {
                    d.audit[..j].iter().rev().find(|e| {
                        e.kind == kind && e.task_id == event.task_id && e.attempt == event.attempt
                    })
                };
                if event.kind == DecisionEventKind::Consumed {
                    ensure!(!d.superseded(), "cannot consume superseded decision");
                } else {
                    ensure!(
                        prior_event(DecisionEventKind::Consumed).is_some(),
                        "decision was never consumed"
                    );
                }
                if matches!(
                    event.kind,
                    DecisionEventKind::ChecksPassed
                        | DecisionEventKind::ReviewAccepted
                        | DecisionEventKind::ReviewRejected
                        | DecisionEventKind::TaskVerified
                ) {
                    ensure!(
                        a.verified_tree.as_ref() == Some(&event.detail)
                            && prior_event(DecisionEventKind::Implemented)
                                .is_some_and(|e| e.detail == event.detail),
                        "decision evidence has a different implementation tree"
                    );
                }
                if matches!(
                    event.kind,
                    DecisionEventKind::ReviewAccepted
                        | DecisionEventKind::ReviewRejected
                        | DecisionEventKind::TaskVerified
                ) {
                    a.review
                        .as_ref()
                        .context("missing decision review")?
                        .validate_decisions(a.decision_ids.as_deref().unwrap_or_default())?;
                    ensure!(
                        prior_event(DecisionEventKind::ChecksPassed)
                            .is_some_and(|e| e.detail == event.detail)
                            && a.review.as_ref().is_some_and(|r| r.accepts()
                                == (event.kind != DecisionEventKind::ReviewRejected)),
                        "decision event lacks corresponding independent review"
                    );
                }
                if event.kind == DecisionEventKind::TaskVerified {
                    ensure!(
                        !d.superseded()
                            && a.preserved_tree == a.verified_tree
                            && prior_event(DecisionEventKind::ReviewAccepted)
                                .is_some_and(|e| e.detail == event.detail),
                        "decision verification lacks matching review/preservation evidence"
                    );
                }
            }
            if event.kind == DecisionEventKind::TaskVerified {
                ensure!(
                    event
                        .task_id
                        .as_ref()
                        .is_some_and(|id| run.tasks.iter().any(|t| &t.plan.id == id
                            && t.status == TaskStatus::Completed
                            && t.attempts
                                .last()
                                .is_some_and(|a| Some(a.number) == event.attempt))),
                    "decision verification requires completed task evidence"
                );
            }
            sqlx::query("INSERT INTO engineering_decision_events(run_id,decision_id,sequence,document) VALUES(?,?,?,?)")
                .bind(run.id.to_string()).bind(&d.id).bind(event.sequence as i64).bind(serde_json::to_string(event)?).execute(&mut **tx).await?;
        }
    }
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
            decisions: vec![],
            id,
            delegation: None,
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
            human_actions: vec![],
            preflight: None,
            integration_workspace: None,
            integration_verification: vec![],
            summary: "Planning".into(),
        }
    }
    fn plan() -> ExecutionPlan {
        ExecutionPlan {
            decisions: vec![],
            human_requirements: vec![],
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
                r.preflight = Some(PreflightReport {
                    checked_at: Utc::now(),
                    checks: vec![],
                    action_ids: vec![],
                    ready: true,
                    limitation: "Fake deterministic fixture".into(),
                });
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
        journal
            .change(
                "human_pause",
                "A task paused before the daemon stopped",
                |r| {
                    r.tasks[0].status = TaskStatus::BlockedHuman;
                    let requirement = HumanRequirement {
                        kind: HumanActionType::Runtime,
                        reason: "Needs external configuration".into(),
                        requested_action: "Configure externally".into(),
                        affected_tasks: vec!["t".into()],
                        blocking: true,
                        evidence: "Fake runtime".into(),
                        confidence: 100,
                        permission: None,
                    };
                    r.human_actions.push(super::super::preflight::action(
                        requirement,
                        "restart-paused",
                        true,
                        Some("snapshot".into()),
                    ));
                    Ok(())
                },
            )
            .await?;
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
        assert_eq!(run.human_actions.len(), 1);
        assert_eq!(run.human_actions[0].status, HumanActionStatus::Pending);
        assert!(!run.status.can_transition(EngineeringStatus::Running));
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
            decision_context: vec![],
            decision_ids: None,
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
                decision_reviews: vec![],
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
            verified_tree: Some("c".repeat(40)),
            preserved_tree: Some("c".repeat(40)),
            detail: None,
        });
        run.revision += 1;
        for (verified, preserved) in [
            (None, None),
            (Some("c".repeat(40)), None),
            (Some("c".repeat(40)), Some("d".repeat(40))),
        ] {
            let mut incomplete = run.clone();
            let attempt = &mut incomplete.tasks[0].attempts[0];
            attempt.verified_tree = verified;
            attempt.preserved_tree = preserved;
            let error = store
                .engineering_save(&incomplete, "completed", "reject missing/mismatched trees")
                .await
                .expect_err("accepted incomplete tree evidence");
            assert!(
                error
                    .to_string()
                    .contains("matching verified/preserved trees")
            );
        }
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
    fn with_delegation(mut run: EngineeringRun) -> EngineeringRun {
        run.specification = "Use feature".into();
        let contract = DelegationContract {
            version: 1,
            request: DelegationProposalRequest {
                submission: PlanSubmission {
                    id: run.id,
                    repository: run.repository.clone(),
                    base: run.requested_base.clone(),
                    specification: run.specification.clone(),
                },
                writable_files: vec!["source.txt".into()],
                max_runtime_seconds: 120,
                local_integration: false,
            },
            repository_git: GitOwnership {
                worktree: run.repository.clone(),
                git_dir: run.repository.join(".git"),
                common_dir: run.repository.join(".git"),
                metadata: Default::default(),
            },
            repository_identity: Default::default(),
            base_commit: run.base_commit.clone(),
            worktree_root: run
                .planning_workspace
                .path
                .parent()
                .expect("parent")
                .to_owned(),
            settings: run.settings.clone(),
            verification_sandbox: "fixture".into(),
            user_config_digest: None,
            limitations: vec![],
        };
        run.delegation = Some(DelegationAuthorization {
            contract,
            authorized_at: run.created_at,
            expires_at: run.created_at + chrono::Duration::seconds(120),
        });
        run
    }
    fn decision() -> DecisionProposal {
        serde_json::from_value(serde_json::json!({"key":"local-default","question":"Which local default?","chosen":"feature","alternatives":["alternate"],"rationale":"Keep the feature local.","kind":"default_value","evidence":[{"file":null,"excerpt":"Use feature","supports":"feature"}],"requirements":["Use feature"],"assumptions":[],"uncertainty":[],"affected_files":["source.txt"],"affected_tasks":["t"],"supersedes":null})).expect("decision")
    }
    #[tokio::test]
    async fn decisions_are_atomic_append_only_idempotent_and_survive_migration_and_reopen()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = std::fs::canonicalize(dir.path())?;
        let db = root.join("db");
        let store = Store::open(&db).await?;
        let run = with_delegation(fixture(&root));
        std::fs::create_dir_all(&run.planning_workspace.path)?;
        let cwd = run.planning_workspace.path.clone();
        store.engineering_create(&run).await?;
        let journal = Journal::new(store.clone(), run.clone());
        let session = Uuid::new_v4();
        // Ledger inserts precede plan validation in the transaction; failure rolls all back.
        assert!(
            journal
                .change("bad", "must roll back", |r| {
                    r.plan = Some(plan());
                    super::super::decisions::record(
                        r,
                        &cwd,
                        &Default::default(),
                        &[decision()],
                        None,
                        None,
                        session,
                    )?;
                    r.plan.as_mut().expect("plan").tasks.clear();
                    Ok(())
                })
                .await
                .is_err()
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM engineering_decisions")
            .fetch_one(&store.readers)
            .await?;
        assert_eq!(count, 0);
        journal
            .change("decision", "persist with plan", |r| {
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
                super::super::decisions::record(
                    r,
                    &cwd,
                    &Default::default(),
                    &[decision()],
                    None,
                    None,
                    session,
                )
            })
            .await?;
        let (a, b) = tokio::join!(
            journal.change("replay", "same proposal", |r| {
                super::super::decisions::record(
                    r,
                    &cwd,
                    &Default::default(),
                    &[decision()],
                    None,
                    None,
                    session,
                )
            }),
            journal.change("replay", "same proposal", |r| {
                super::super::decisions::record(
                    r,
                    &cwd,
                    &Default::default(),
                    &[decision()],
                    None,
                    None,
                    session,
                )
            })
        );
        a?;
        b?;
        let saved = journal.read().await;
        assert_eq!(saved.decisions.len(), 1);
        assert_eq!(saved.decisions[0].audit.len(), 1);
        assert!(
            journal
                .change(
                    "rekey",
                    "same scoped ambiguity cannot duplicate roots",
                    |r| {
                        let mut p = decision();
                        p.key = "different-key".into();
                        p.question = " WHICH   LOCAL DEFAULT? ".into();
                        super::super::decisions::record(
                            r,
                            &cwd,
                            &Default::default(),
                            &[p],
                            None,
                            None,
                            session,
                        )
                    }
                )
                .await
                .is_err()
        );
        assert!(
            journal
                .change("unrelated_replay", "must retain task attribution", |r| {
                    super::super::decisions::record(
                        r,
                        &cwd,
                        &Default::default(),
                        &[decision()],
                        Some("other"),
                        Some(1),
                        session,
                    )
                })
                .await
                .is_err()
        );
        assert!(
            journal
                .change("contradictory_replay", "must explicitly supersede", |r| {
                    let mut p = decision();
                    p.chosen = "changed".into();
                    super::super::decisions::record(
                        r,
                        &cwd,
                        &Default::default(),
                        &[p],
                        None,
                        None,
                        session,
                    )
                })
                .await
                .is_err()
        );
        assert!(
            journal
                .change("rewrite", "forbidden", |r| {
                    r.decisions[0].proposal.chosen = "changed".into();
                    Ok(())
                })
                .await
                .is_err()
        );
        assert!(
            journal
                .change("erase", "forbidden", |r| {
                    r.decisions.clear();
                    Ok(())
                })
                .await
                .is_err()
        );
        assert!(
            journal
                .change("forge", "forbidden", |r| {
                    r.decisions[0].audit.push(DecisionEvent {
                        sequence: 2,
                        at: Utc::now(),
                        kind: DecisionEventKind::TaskVerified,
                        task_id: Some("t".into()),
                        attempt: Some(1),
                        detail: "forged".into(),
                    });
                    Ok(())
                })
                .await
                .is_err()
        );
        for table in ["engineering_decisions", "engineering_decision_events"] {
            assert!(
                sqlx::query(&format!("UPDATE {table} SET document='{{}}'"))
                    .execute(&store.pool)
                    .await
                    .is_err()
            );
            assert!(
                sqlx::query(&format!("DELETE FROM {table}"))
                    .execute(&store.pool)
                    .await
                    .is_err()
            );
        }
        journal
            .change("attempt_started", "freeze consumed revisions", |r| {
                let decision_context = super::super::decisions::active_ids(r, "t");
                r.tasks[0].attempts.push(TaskAttempt {
                    number: 1,
                    decision_context,
                    decision_ids: None,
                    coder: AgentSession {
                        id: session,
                        role: AgentRole::Coder,
                        provider: Provider::Claude,
                        provider_session_id: None,
                        resumed_from: None,
                        process_key: Uuid::new_v4(),
                        process_run_id: None,
                        outcome: None,
                        result: None,
                    },
                    verification: vec![],
                    reviewer: None,
                    reviews: vec![],
                    review: None,
                    snapshot: None,
                    verified_tree: None,
                    preserved_tree: None,
                    detail: None,
                });
                Ok(())
            })
            .await?;
        journal
            .change("consume", "bind before implementation", |r| {
                super::super::decisions::consume(r, "t", 1)
            })
            .await?;
        journal
            .change("consume_replay", "idempotent", |r| {
                super::super::decisions::consume(r, "t", 1)
            })
            .await?;
        let bound = journal.read().await;
        let mut historical = bound.decisions[0].clone();
        historical.evidence[0].source = DecisionEvidenceSource::LegacyUnverified;
        assert!(!super::super::decisions::trusted_basis(&bound, &historical));
        assert!(bound.decisions[0].autonomous());
        assert_eq!(
            bound.tasks[0].attempts[0].decision_ids,
            Some(vec![bound.decisions[0].id.clone()])
        );
        assert_eq!(
            bound.decisions[0]
                .audit
                .iter()
                .filter(|e| e.kind == DecisionEventKind::Consumed)
                .count(),
            1
        );
        journal
            .change("revision", "preserve supersession", |r| {
                let mut p = decision();
                p.supersedes = Some(r.decisions[0].id.clone());
                p.chosen = "alternate".into();
                p.alternatives = vec!["feature".into()];
                p.evidence.clear();
                super::super::decisions::record(
                    r,
                    &cwd,
                    &Default::default(),
                    &[p],
                    None,
                    None,
                    session,
                )
            })
            .await?;
        assert!(
            journal
                .change("stale_replay", "reject retired revision", |r| {
                    super::super::decisions::record(
                        r,
                        &cwd,
                        &Default::default(),
                        &[decision()],
                        None,
                        None,
                        session,
                    )
                })
                .await
                .is_err()
        );
        for context in [true, false] {
            assert!(
                journal
                    .change("rebind", "cannot change consumed revision", |r| {
                        let replacement = vec![r.decisions[1].id.clone()];
                        if context {
                            r.tasks[0].attempts[0].decision_context = replacement;
                        } else {
                            r.tasks[0].attempts[0].decision_ids = Some(replacement);
                        }
                        Ok(())
                    })
                    .await
                    .is_err()
            );
        }
        assert!(
            journal
                .change(
                    "misattribute",
                    "cannot append evidence to another revision",
                    |r| {
                        r.decisions[1].audit.push(DecisionEvent {
                            sequence: 2,
                            at: Utc::now(),
                            kind: DecisionEventKind::Implemented,
                            task_id: Some("t".into()),
                            attempt: Some(1),
                            detail: "tree".into(),
                        });
                        Ok(())
                    }
                )
                .await
                .is_err()
        );
        journal
            .change(
                "implementation",
                "historical events retain exact binding",
                |r| {
                    super::super::decisions::event(
                        r,
                        "t",
                        1,
                        DecisionEventKind::Implemented,
                        "tree",
                    );
                    Ok(())
                },
            )
            .await?;
        let saved = journal.read().await;
        assert!(
            saved.decisions[0]
                .audit
                .iter()
                .any(|e| e.kind == DecisionEventKind::Implemented)
        );
        assert!(
            !saved.decisions[1]
                .audit
                .iter()
                .any(|e| e.kind == DecisionEventKind::Implemented)
        );
        store.pool.close().await;
        store.readers.close().await;
        let reopened = Store::open(&db).await?;
        assert_eq!(
            reopened.engineering_get(run.id).await?.decisions,
            saved.decisions
        );
        let stored: String = sqlx::query_scalar("SELECT document FROM engineering_decisions")
            .fetch_one(&reopened.readers)
            .await?;
        let identity: DecisionRecord = serde_json::from_str(&stored)?;
        assert!(identity.audit.is_empty());
        let events: i64 = sqlx::query_scalar("SELECT count(*) FROM engineering_decision_events")
            .fetch_one(&reopened.readers)
            .await?;
        assert_eq!(
            events as usize,
            saved.decisions.iter().map(|d| d.audit.len()).sum::<usize>()
        );
        assert_eq!(
            serde_json::to_value(
                reopened.engineering_get(run.id).await?.tasks[0].attempts[0].clone()
            )?,
            serde_json::to_value(&saved.tasks[0].attempts[0])?
        );
        Ok(())
    }
    #[tokio::test]
    async fn a3_migration_preserves_legacy_supervised_documents() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = dir.path().join("db");
        let store = Store::open(&db).await?;
        let run = fixture(dir.path());
        store.engineering_create(&run).await?;
        let mut legacy = serde_json::to_value(&run)?;
        legacy.as_object_mut().expect("run").remove("decisions");
        sqlx::query("UPDATE engineering_runs SET document=? WHERE id=?")
            .bind(serde_json::to_string(&legacy)?)
            .bind(run.id.to_string())
            .execute(&store.pool)
            .await?;
        sqlx::query("DROP TABLE engineering_decision_events")
            .execute(&store.pool)
            .await?;
        sqlx::query("DROP TABLE engineering_decisions")
            .execute(&store.pool)
            .await?;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version=7")
            .execute(&store.pool)
            .await?;
        store.pool.close().await;
        store.readers.close().await;
        let upgraded = Store::open(&db).await?;
        let restored = upgraded.engineering_get(run.id).await?;
        assert!(restored.decisions.is_empty());
        assert!(restored.delegation.is_none());
        assert_eq!(restored.specification, run.specification);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM engineering_decisions")
            .fetch_one(&upgraded.readers)
            .await?;
        assert_eq!(count, 0);
        Ok(())
    }
}
