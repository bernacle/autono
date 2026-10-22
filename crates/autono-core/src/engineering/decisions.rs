//! Conservative, non-authorizing decision assessment. No provider calls or new reads.
use super::*;
use anyhow::{Result, ensure};
use chrono::Utc;
use std::{collections::BTreeSet, path::Path};
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

pub(super) fn record(
    run: &mut EngineeringRun,
    cwd: &Path,
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
            ensure!(existing.proposal == *p, invalid());
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
            let content = if let Some(file) = &e.file {
                // Do not even probe an unauthorized path.
                if auth.contract.request.writable_files.contains(file) {
                    scoped_files::read(cwd, file).ok().flatten()
                } else {
                    None
                }
            } else {
                Some(run.specification.clone())
            };
            if let Some(content) = content.filter(|content| content.contains(&e.excerpt)) {
                evidence.push(DecisionProvenance {
                    file: e.file.clone(),
                    content_sha256: review::digest(&content),
                    excerpt: e.excerpt.clone(),
                    supports: e.supports.clone(),
                    base_commit: run.base_commit.clone(),
                    workspace_id: if let Some(task) = task_id {
                        run.tasks
                            .iter()
                            .find(|t| t.plan.id == task)
                            .and_then(|t| t.workspace.as_ref())
                            .map(|w| w.id)
                            .ok_or_else(invalid)?
                    } else {
                        run.planning_workspace.id
                    },
                });
            } else {
                reliable = false;
            }
            contradictory |= e.supports != p.chosen;
        }
        let requirements_known = p.requirements.iter().all(|r| run.specification.contains(r));
        // Deliberately conservative lexical conflict gate; semantic conflicts also
        // require independent review against the complete original specification.
        let explicit_alternative = p
            .alternatives
            .iter()
            .any(|a| run.specification.to_lowercase().contains(&a.to_lowercase()))
            && !run
                .specification
                .to_lowercase()
                .contains(&p.chosen.to_lowercase());
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
                || explicit_alternative
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

pub(super) fn event(
    run: &mut EngineeringRun,
    task: &str,
    attempt: usize,
    kind: DecisionEventKind,
    detail: &str,
) {
    for d in &mut run.decisions {
        if d.autonomous() && !d.superseded() && d.proposal.affected_tasks.iter().any(|t| t == task)
        {
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
    "\nA3 decisions: propose a decisions array (empty when none) for genuine ambiguities. Minor local patterns, internal names, defaults and test structures may be resolved using exact excerpts from authorized files or the original specification. Include alternatives, concise rationale (never private reasoning), exact original requirement excerpts, assumptions, uncertainty and affected task/file IDs. Evidence supports names the supported option. file=null means original specification. Missing context, conflicting requirements, architecture/compatibility changes and unknown effects remain unresolved; report these honestly. kind is local_pattern, internal_name, default_value, test_structure, significant, authorization, unknown_effects or prerequisite. Never reclassify a permission, prerequisite, unknown effect or explicit human requirement as a preference. A decision grants no permissions, file access, commands or deadline extensions. Reuse the same key/payload on replay; corrections use supersedes with the previous decision ID. All choices remain subject to configured checks and independent review. Reviewers must independently challenge the choices and compare original requirements; do not emit new decisions.\n"
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
