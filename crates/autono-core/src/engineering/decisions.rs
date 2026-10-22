//! Conservative, non-authorizing assessment using exact immutable Git objects.
use super::*;
use anyhow::{Result, ensure};
use chrono::Utc;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn invalid() -> anyhow::Error {
    agent::AgentFailure { kind: "invalid_decision".into(), detail: "Decision contract is malformed, oversized or contains credential-like material; inspect the task without expanding authority".into(), blocked: true }.into()
}
fn safe(s: &str) -> Result<()> {
    ensure!(
        s.chars().all(|c| !c.is_control() || c == '\n' || c == '\t'),
        invalid()
    );
    preflight::safe_text(s).map_err(|_| invalid())?;
    // Match key prefixes at token boundaries; ordinary task IDs such as
    // task-export contain "sk-" and must not be mistaken for credentials.
    ensure!(
        !s.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .any(|token| token.starts_with("sk-") || token.starts_with("SK-")),
        invalid()
    );
    let compact: String = s
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect();
    ensure!(
        ![
            "ghp_",
            "github_pat_",
            "akia",
            "password:",
            "password=",
            "token=",
            "secret=",
            "private_key:",
            "private_key=",
            "token:",
            "secret:",
            "api_key:",
            "api-key:",
            "apikey:",
            "api_key=",
            "api-key=",
            "apikey=",
            "credential:",
            "credential=",
            "authorization:"
        ]
        .iter()
        .any(|v| compact.contains(v)),
        invalid()
    );
    Ok(())
}
/// Validate before retaining provider result in a session. Diagnostics never echo payloads.
pub(super) fn parse(value: Option<&serde_json::Value>) -> Result<Vec<DecisionProposal>> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    ensure!(serde_json::to_vec(value)?.len() <= 32 * 1024, invalid());
    let proposals: Vec<DecisionProposal> =
        serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    ensure!(proposals.len() <= 8, invalid());
    let mut keys = BTreeSet::new();
    for p in &proposals {
        ensure!(
            !p.key.is_empty()
                && p.key.len() <= 64
                && p.key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && keys.insert(&p.key),
            invalid()
        );
        ensure!(
            (1..=8).contains(&p.alternatives.len())
                && p.evidence.len() <= 8
                && (1..=8).contains(&p.requirements.len())
                && p.assumptions.len() <= 8
                && p.uncertainty.len() <= 8
                && (1..=128).contains(&p.affected_files.len())
                && (1..=32).contains(&p.affected_tasks.len()),
            invalid()
        );
        for s in [&p.question, &p.chosen, &p.rationale]
            .into_iter()
            .chain(p.alternatives.iter())
            .chain(p.requirements.iter())
            .chain(p.assumptions.iter())
            .chain(p.uncertainty.iter())
            .chain(p.affected_files.iter())
            .chain(p.affected_tasks.iter())
            .chain(p.supersedes.iter())
        {
            safe(s)?;
        }
        ensure!(
            p.alternatives.iter().all(|a| a != &p.chosen)
                && p.alternatives.iter().collect::<BTreeSet<_>>().len() == p.alternatives.len()
                && p.affected_files.iter().collect::<BTreeSet<_>>().len() == p.affected_files.len()
                && p.affected_tasks.iter().collect::<BTreeSet<_>>().len() == p.affected_tasks.len(),
            invalid()
        );
        for e in &p.evidence {
            safe(&e.excerpt)?;
            safe(&e.supports)?;
            if let Some(file) = &e.file {
                safe(file)?;
            }
            ensure!(
                e.supports == p.chosen || p.alternatives.contains(&e.supports),
                invalid()
            );
        }
    }
    Ok(proposals)
}

pub(super) fn review_content(value: Option<&serde_json::Value>) -> Result<()> {
    if let Some(value) = value {
        ensure!(serde_json::to_vec(value)?.len() <= 32 * 1024, invalid());
        let reviews: Vec<DecisionReview> =
            serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        ensure!(reviews.len() <= 128, invalid());
        for review in reviews {
            safe(&review.decision_id)?;
            safe(&review.evidence)?;
        }
    }
    Ok(())
}

pub(super) struct FrozenSource {
    content: String,
    blob_oid: String,
}

/// Read only cited, exactly allowlisted paths from the original pinned base.
/// Mutable workspaces (including failed attempts and dependency artifacts) are
/// candidate material, never established project convention in this policy.
pub(super) async fn sources(
    run: &EngineeringRun,
    runner: &process::ProcessRunner,
    cwd: &Path,
    proposals: &[DecisionProposal],
    stop: &CancellationToken,
) -> Result<BTreeMap<String, FrozenSource>> {
    let mut sources = BTreeMap::new();
    let Some(auth) = &run.delegation else {
        return Ok(sources);
    };
    delegation::owns_cwd(&auth.contract, cwd)?;
    let files: BTreeSet<_> = proposals
        .iter()
        .flat_map(|p| &p.evidence)
        .filter_map(|e| e.file.as_ref())
        .filter(|f| auth.contract.request.writable_files.contains(f))
        .collect();
    let mut total = 0;
    for file in files {
        let pathspec = format!(":(literal){file}");
        let entry = runner
            .git(
                cwd,
                &[
                    "ls-tree",
                    "-l",
                    "-z",
                    "--full-tree",
                    &run.base_commit,
                    "--",
                    &pathspec,
                ],
                stop,
            )
            .await?;
        let Some((metadata, path)) = entry.strip_suffix('\0').and_then(|s| s.split_once('\t'))
        else {
            continue;
        };
        let fields: Vec<_> = metadata.split_whitespace().collect();
        if path != file
            || fields.len() != 4
            || !matches!(fields[0], "100644" | "100755")
            || fields[1] != "blob"
            || fields[3]
                .parse::<usize>()
                .ok()
                .is_none_or(|s| s > 256 * 1024)
        {
            continue;
        }
        let content = runner
            .git(cwd, &["cat-file", "blob", fields[2]], stop)
            .await?;
        total += content.len();
        ensure!(total <= 1024 * 1024, invalid());
        ensure!(content.len() == fields[3].parse::<usize>()?, invalid());
        sources.insert(
            file.clone(),
            FrozenSource {
                content,
                blob_oid: fields[2].into(),
            },
        );
    }
    Ok(sources)
}

/// This is intentionally a small lexical gate, not semantic understanding.
/// Any competing mention, quotation, negation, contrast or unfamiliar wording
/// stays unresolved. A sole option mention needs a simple positive instruction.
fn requirements_compatible(specification: &str, p: &DecisionProposal) -> bool {
    let spec = specification.to_lowercase();
    let chosen = p.chosen.to_lowercase();
    if p.alternatives
        .iter()
        .any(|a| spec.contains(&a.to_lowercase()))
    {
        return false;
    }
    if !spec.contains(&chosen) {
        return true;
    }
    let clauses: Vec<_> = spec
        .split(['.', '\n', ';', '!', '?'])
        .filter(|s| s.contains(&chosen))
        .collect();
    clauses.len() == 1 && {
        let clause = clauses[0].trim();
        clause == format!("use {chosen}")
            || clause == format!("use {chosen} helper names")
            || clause == format!("use {chosen} as the default export format")
    }
}

pub(super) fn record(
    run: &mut EngineeringRun,
    cwd: &Path,
    sources: &BTreeMap<String, FrozenSource>,
    proposals: &[DecisionProposal],
    task_id: Option<&str>,
    attempt: Option<usize>,
    session_id: Uuid,
) -> Result<()> {
    let auth = run.delegation.as_ref().ok_or_else(invalid)?;
    delegation::owns_cwd(&auth.contract, cwd)?;
    let plan = run.plan.as_ref().ok_or_else(invalid)?;
    for p in proposals {
        ensure!(
            p.affected_tasks
                .iter()
                .all(|t| plan.tasks.iter().any(|x| &x.id == t))
                && task_id.is_none_or(|t| p.affected_tasks.iter().any(|x| x == t)),
            invalid()
        );
        // Rekeying the same scoped question must not create contradictory roots.
        // Semantic paraphrases still require review; no inference router is added.
        let question = p
            .question
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        ensure!(
            !run.decisions.iter().any(|d| !d.superseded()
                && d.proposal.key != p.key
                && d.proposal
                    .question
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase()
                    == question
                && d.proposal
                    .affected_tasks
                    .iter()
                    .any(|t| p.affected_tasks.contains(t))),
            invalid()
        );
        // Key plus predecessor identifies a revision; matching replay is a no-op.
        let id = format!(
            "D-{}",
            review::digest(&serde_json::to_string(&(run.id, &p.key, &p.supersedes))?)
        );
        if let Some(existing) = run.decisions.iter().find(|d| d.id == id) {
            ensure!(existing.proposal == *p && !existing.superseded(), invalid());
            continue;
        }
        ensure!(run.decisions.len() < 128, invalid());
        // A running task may revise itself, never race another task's choice.
        let busy = run.tasks.iter().any(|t| {
            p.affected_tasks.contains(&t.plan.id)
                && (t.status == TaskStatus::Completed
                    || (t.status == TaskStatus::Running && Some(t.plan.id.as_str()) != task_id))
        });
        let previous = run
            .decisions
            .iter()
            .find(|d| d.proposal.key == p.key && !d.superseded());
        ensure!(
            match (&p.supersedes, previous) {
                (None, None) => true,
                (Some(id), Some(old)) =>
                    id == &old.id && old.proposal.affected_tasks == p.affected_tasks,
                _ => false,
            },
            invalid()
        );
        let files_allowed = p
            .affected_files
            .iter()
            .all(|f| auth.contract.request.writable_files.contains(f));
        let mut category = match p.kind {
            DecisionKind::Authorization | DecisionKind::UnknownEffects => {
                DecisionCategory::Authorization
            }
            DecisionKind::Significant | DecisionKind::Prerequisite => DecisionCategory::Significant,
            _ => DecisionCategory::Reversible,
        };
        let mut basis = "Existing immutable A2 delegation only: exact local files, no executable decision operations; independent checks and review required".to_string();
        let evidence_allowed = p.evidence.iter().all(|e| {
            e.file
                .as_ref()
                .is_none_or(|f| auth.contract.request.writable_files.contains(f))
        });
        if !files_allowed || !evidence_allowed {
            category = DecisionCategory::Authorization;
            basis = "Affected files or evidence references exceed the exact A2 allowlist; a decision cannot grant access"
                .into();
        }
        let mut evidence = vec![];
        let mut reliable = !p.evidence.is_empty()
            && p.evidence.iter().any(|e| {
                e.supports == p.chosen
                    && e.excerpt.to_lowercase().contains(&p.chosen.to_lowercase())
            });
        let mut contradictory = false;
        for e in &p.evidence {
            let frozen = e.file.as_ref().and_then(|f| sources.get(f));
            let content = if e.file.is_none() {
                Some(&run.specification)
            } else {
                frozen.map(|s| &s.content)
            };
            if let Some(content) = content.filter(|content| content.contains(&e.excerpt)) {
                evidence.push(DecisionProvenance {
                    source: if e.file.is_none() {
                        DecisionEvidenceSource::Specification
                    } else {
                        DecisionEvidenceSource::FrozenBase
                    },
                    file: e.file.clone(),
                    content_sha256: review::digest(content),
                    excerpt: e.excerpt.clone(),
                    supports: e.supports.clone(),
                    base_commit: frozen.map(|_| run.base_commit.clone()),
                    workspace_id: None,
                    blob_oid: frozen.map(|s| s.blob_oid.clone()),
                });
            } else {
                reliable = false;
            }
            contradictory |= e.supports != p.chosen;
        }
        let requirements_known = p.requirements.iter().all(|r| run.specification.contains(r));
        let compatible = requirements_compatible(&run.specification, p);
        let human_gate = plan.human_requirements.iter().any(|h| {
            h.blocking
                && (h.affected_tasks.is_empty()
                    || h.affected_tasks
                        .iter()
                        .any(|t| p.affected_tasks.contains(t)))
        });
        if category == DecisionCategory::Reversible
            && (!reliable
                || contradictory
                || !requirements_known
                || !p.uncertainty.is_empty()
                || !compatible
                || human_gate
                || busy)
        {
            category = DecisionCategory::Significant;
            basis = "Unresolved: missing/contradictory authorized evidence, uncertain context, requirement conflict, existing human gate or concurrent/completed work".into();
        }
        if category != DecisionCategory::Reversible && basis.starts_with("Existing") {
            basis = "Significant choice, prerequisite, authorization or unknown effects require the existing human workflow; no operation is authorized by this record".into();
        }
        let now = Utc::now();
        if let Some(old_id) = &p.supersedes {
            let old = run
                .decisions
                .iter_mut()
                .find(|d| &d.id == old_id)
                .ok_or_else(invalid)?;
            old.audit.push(DecisionEvent {
                sequence: old.audit.len() + 1,
                at: now,
                kind: DecisionEventKind::Superseded,
                task_id: task_id.map(String::from),
                attempt,
                detail: id.clone(),
            });
        }
        let accepted = category == DecisionCategory::Reversible;
        run.decisions.push(DecisionRecord {
            id: id.clone(),
            run_id: run.id,
            ordinal: run.decisions.len() + 1,
            task_id: task_id.map(String::from),
            attempt,
            session_id,
            created_at: now,
            proposal: p.clone(),
            category,
            policy_basis: basis.clone(),
            evidence,
            audit: vec![DecisionEvent {
                sequence: 1,
                at: now,
                kind: if accepted {
                    DecisionEventKind::Accepted
                } else {
                    DecisionEventKind::Unresolved
                },
                task_id: task_id.map(String::from),
                attempt,
                detail: basis,
            }],
        });
        if !accepted {
            let req = HumanRequirement {
                kind: if category == DecisionCategory::Authorization { HumanActionType::Permission } else if p.kind == DecisionKind::Prerequisite { HumanActionType::Environment } else { HumanActionType::ProductDecision },
                reason: format!("Unresolved decision {id}: {}", p.question),
                affected_tasks: p.affected_tasks.clone(),
                requested_action: "Inspect the decision ledger and original requirements. Authorization/external effects require the existing permission/reconciliation workflow; textual decisions cannot expand the contract.".into(),
                blocking: true, evidence: format!("Daemon decision assessment: {id}"), confidence: 100,
                permission: if category == DecisionCategory::Authorization { Some(PermissionRequest {
                    operation: if !files_allowed { PermissionOperation::OutsideWorkspace } else { PermissionOperation::ExternalOperation },
                    scope: format!("Decision {id}: operation exceeds immutable A2 authority; use external reconciliation or a new contract"),
                }) } else { None },
            };
            run.human_actions
                .push(preflight::action(req, &id, false, None));
        }
    }
    Ok(())
}

pub(super) fn active_ids(run: &EngineeringRun, task: &str) -> Vec<String> {
    run.decisions
        .iter()
        .filter(|d| {
            d.autonomous() && !d.superseded() && d.proposal.affected_tasks.iter().any(|t| t == task)
        })
        .map(|d| d.id.clone())
        .collect()
}

pub(super) fn trusted_basis(run: &EngineeringRun, d: &DecisionRecord) -> bool {
    !d.evidence.is_empty()
        && d.evidence.iter().all(|e| match e.source {
            DecisionEvidenceSource::FrozenBase => {
                e.base_commit.as_ref() == Some(&run.base_commit)
                    && e.blob_oid.is_some()
                    && e.file.is_some()
            }
            DecisionEvidenceSource::Specification => {
                e.file.is_none() && e.content_sha256 == review::digest(&run.specification)
            }
            DecisionEvidenceSource::LegacyUnverified => false,
        })
}

/// Freeze the revisions this response consumed before applying any edits. The
/// only changes from the invocation's context may be its own admitted revisions.
pub(super) fn consume(run: &mut EngineeringRun, task: &str, attempt: usize) -> Result<()> {
    let ids = active_ids(run, task);
    ensure!(
        run.decisions
            .iter()
            .filter(|d| ids.contains(&d.id))
            .all(|d| trusted_basis(run, d)),
        invalid()
    );
    let a = run
        .tasks
        .iter()
        .find(|t| t.plan.id == task)
        .and_then(|t| t.attempts.iter().find(|a| a.number == attempt))
        .ok_or_else(invalid)?;
    if let Some(previous) = &a.decision_ids {
        ensure!(previous == &ids, invalid());
        return Ok(());
    }
    for id in &a.decision_context {
        ensure!(
            ids.contains(id)
                || run
                    .decisions
                    .iter()
                    .any(|d| d.proposal.supersedes.as_ref() == Some(id)
                        && d.task_id.as_deref() == Some(task)
                        && d.attempt == Some(attempt)
                        && ids.contains(&d.id)),
            invalid()
        );
    }
    ensure!(
        ids.iter().all(|id| a.decision_context.contains(id)
            || run.decisions.iter().any(|d| &d.id == id
                && d.task_id.as_deref() == Some(task)
                && d.attempt == Some(attempt))),
        invalid()
    );
    run.tasks
        .iter_mut()
        .find(|t| t.plan.id == task)
        .ok_or_else(invalid)?
        .attempts
        .iter_mut()
        .find(|a| a.number == attempt)
        .ok_or_else(invalid)?
        .decision_ids = Some(ids);
    event(
        run,
        task,
        attempt,
        DecisionEventKind::Consumed,
        "Admitted coder response; exact decision revisions frozen before file replacements",
    );
    Ok(())
}

pub(super) fn event(
    run: &mut EngineeringRun,
    task: &str,
    attempt: usize,
    kind: DecisionEventKind,
    detail: &str,
) {
    let ids = run
        .tasks
        .iter()
        .find(|t| t.plan.id == task)
        .and_then(|t| t.attempts.iter().find(|a| a.number == attempt))
        .and_then(|a| a.decision_ids.as_ref())
        .cloned()
        .unwrap_or_default();
    for d in &mut run.decisions {
        if ids.contains(&d.id) {
            // Journal checkpoint retries/replay cannot duplicate an audit fact.
            if !d.audit.iter().any(|e| {
                e.kind == kind && e.task_id.as_deref() == Some(task) && e.attempt == Some(attempt)
            }) {
                d.audit.push(DecisionEvent {
                    sequence: d.audit.len() + 1,
                    at: Utc::now(),
                    kind,
                    task_id: Some(task.into()),
                    attempt: Some(attempt),
                    detail: detail.into(),
                });
            }
        }
    }
}

pub(super) fn instructions() -> &'static str {
    "\nA3 decisions: propose a decisions array (empty when none) for genuine ambiguities. Minor local patterns, internal names, defaults and test structures may be resolved using exact excerpts from the original frozen-base versions of authorized files or the original specification. Current workspace bytes, including failed or dependency artifacts, are candidate context and cannot establish project conventions. Explicit requirements override conventions; an option mentioned in a prohibition or example is not permission. Include alternatives, concise rationale (never private reasoning), exact original requirement excerpts, assumptions, uncertainty and affected task/file IDs. Evidence supports names the supported option. file=null means original specification. Missing context, conflicting requirements, architecture/compatibility changes and unknown effects remain unresolved; report these honestly. kind is local_pattern, internal_name, default_value, test_structure, significant, authorization, unknown_effects or prerequisite. Never reclassify a permission, prerequisite, unknown effect or explicit human requirement as a preference. A decision grants no permissions, file access, commands or deadline extensions. Reuse the same key/payload only while its revision is active; superseded replay is rejected. Corrections or reverting to a historical choice require a new revision with supersedes naming the current decision ID. All choices remain subject to configured checks and independent review. Reviewers must independently challenge the choices and compare original requirements; do not emit new decisions.\n"
}
pub(super) fn schema() -> serde_json::Value {
    use serde_json::json;
    let string = json!({"type":"string"});
    let strings = json!({"type":"array","items":string});
    json!({"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,
        "required":["key","question","chosen","alternatives","rationale","kind","evidence","requirements","assumptions","uncertainty","affected_files","affected_tasks","supersedes"],
        "properties":{"key":string,"question":string,"chosen":string,"alternatives":strings,"rationale":string,
            "kind":{"type":"string","enum":["local_pattern","internal_name","default_value","test_structure","significant","authorization","unknown_effects","prerequisite"]},
            "evidence":{"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,"required":["file","excerpt","supports"],"properties":{"file":{"type":["string","null"]},"excerpt":string,"supports":string}}},
            "requirements":strings,"assumptions":strings,"uncertainty":strings,"affected_files":strings,"affected_tasks":strings,"supersedes":{"type":["string","null"]}}
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requirement_mentions_are_not_permission_or_semantic_proof() {
        let p: DecisionProposal = serde_json::from_value(serde_json::json!({"key":"name","question":"Which helper?","chosen":"stable","alternatives":["legacy"],"rationale":"Reuse naming","kind":"internal_name","evidence":[],"requirements":["Implement"],"assumptions":[],"uncertainty":[],"affected_files":["b.txt"],"affected_tasks":["b"],"supersedes":null})).expect("proposal");
        for spec in [
            "Use legacy helper names. Do not use stable helper names.",
            "Do not use stable helper names.",
            "Never use stable.",
            "Avoid stable helper names.",
            "Use no stable helper names.",
            "Don't use stable helper names.",
            "Example: 'Use stable helper names'.",
            "Example: \"stable\".",
            "Use legacy rather than stable.",
            "Consider stable versus legacy.",
            "Use stable helper names. Use legacy helper names.",
            "Use stable helper names. Do not use stable helper names.",
            "Use stable helper names unless compatibility requires otherwise.",
        ] {
            assert!(
                !requirements_compatible(spec, &p),
                "ambiguous/prohibited: {spec}"
            );
        }
        for spec in [
            "Implement safe feature",
            "Implement safe feature. Use stable helper names.",
            "Use stable.",
        ] {
            assert!(requirements_compatible(spec, &p), "unambiguous: {spec}");
        }
    }
    #[test]
    fn decision_content_rejects_credentials_and_malformed_contracts_without_echoing() {
        for value in [
            "sk-fixture-1234567890",
            "A bearer canary-value",
            "api_key: canary-value",
            "password = canary-value",
            "token=canary-value",
            "secret : canary-value",
            "ghp_fixture123456",
            "-----BEGIN PRIVATE KEY-----",
            "hidden\u{1b}[2Jterminal-control",
        ] {
            assert!(safe(value).is_err(), "must reject credential-like input");
        }
        assert!(review_content(Some(&serde_json::json!([{"decision_id":"D-fixture","implemented":true,"evidence":"api_key: canary-value"}]))).is_err());
        assert!(safe("task-export").is_ok());
        assert!(safe("src/mask-utils.rs").is_ok());
        assert!(safe("Use (sk-fixture-1234567890)").is_err());
        assert!(parse(None).expect("legacy output").is_empty());
        for value in [
            serde_json::json!(null),
            serde_json::json!([{"key":"missing-fields"}]),
            serde_json::json!({"question":"not-array"}),
        ] {
            let error = parse(Some(&value)).expect_err("malformed");
            assert!(!error.to_string().contains("missing-fields"));
        }
    }
}
