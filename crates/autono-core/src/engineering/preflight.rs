//! Deterministic host checks and material planner proposals. No credential values are exposed or retained.
use super::{agent::AgentRuntime, journal::Journal, process::ProcessRunner, *};
use anyhow::{Result, ensure};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use tokio_util::sync::CancellationToken;

fn stable_id(key: &str) -> String {
    format!("H-{:x}", Sha256::digest(key.as_bytes()))[..34].into()
}

pub(super) fn action(
    requirement: HumanRequirement,
    key: &str,
    continuable: bool,
    snapshot: Option<String>,
) -> HumanAction {
    let now = Utc::now();
    let category =
        if requirement.permission.is_some() || requirement.kind == HumanActionType::Runtime {
            PrerequisiteCategory::UnsafeOrUncertain
        } else if requirement.affected_tasks.is_empty() {
            PrerequisiteCategory::EssentialBeforeStart
        } else {
            PrerequisiteCategory::DeferrableManualDependency
        };
    HumanAction {
        prerequisite: Some(PrerequisiteClassification {
            category,
            basis: "Daemon classification from exact action kind and frozen task scope; no model assertion grants authority".into(),
            alternative_decision_id: None,
            verification_remaining: vec!["Original task checks, fresh independent review and combined integration checks remain required".into()],
        }),
        id: stable_id(key),
        requirement,
        status: HumanActionStatus::Pending,
        resolution: None,
        created_at: now,
        updated_at: now,
        audit: vec![ActionAudit {
            at: now,
            status: HumanActionStatus::Pending,
            request: None,
        }],
        continuable,
        workspace_snapshot: snapshot,
    }
}
fn requirement(
    kind: HumanActionType,
    reason: String,
    requested_action: String,
) -> HumanRequirement {
    HumanRequirement {
        kind,
        reason,
        requested_action,
        affected_tasks: vec![],
        blocking: true,
        evidence: "Daemon host check; values were not read".into(),
        confidence: 100,
        permission: None,
    }
}
pub(super) fn available(program: &str, cwd: &Path) -> bool {
    let executable = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                m.is_file()
            }
        })
    };
    if Path::new(program).components().count() > 1 || Path::new(program).is_absolute() {
        return executable(&cwd.join(program));
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|p| executable(&cwd.join(p).join(program)))
    })
}
fn planner_requirements(
    proposed: &[HumanRequirement],
    existing: &[HumanAction],
) -> Vec<HumanRequirement> {
    // Stable hashes predate A4. Preserve their exact original grouping/order on
    // upgrade; no new question may replace an already accepted legacy action.
    if existing
        .iter()
        .any(|a| a.prerequisite.is_none() && proposed.contains(&a.requirement))
    {
        return proposed.to_vec();
    }
    let mut requirements: Vec<HumanRequirement> = vec![];
    for proposed in proposed {
        let mut identity = proposed.clone();
        identity.affected_tasks.clear();
        if let Some(existing) = requirements.iter_mut().find(|r| {
            let mut r = (*r).clone();
            r.affected_tasks.clear();
            r == identity
        }) {
            // An empty scope means run-wide; merging cannot narrow it.
            if existing.affected_tasks.is_empty() || proposed.affected_tasks.is_empty() {
                existing.affected_tasks.clear();
            } else {
                existing
                    .affected_tasks
                    .extend(proposed.affected_tasks.clone());
                existing.affected_tasks.sort();
                existing.affected_tasks.dedup();
            }
        } else {
            let mut proposed = proposed.clone();
            proposed.affected_tasks.sort();
            proposed.affected_tasks.dedup();
            requirements.push(proposed);
        }
    }
    requirements
}

pub(super) async fn inspect(
    journal: &Journal,
    runner: &ProcessRunner,
    agents: &Arc<dyn AgentRuntime>,
    stop: &CancellationToken,
) -> Result<()> {
    runner.check_authority(stop)?;
    let run = journal.read().await;
    let mut checks = Vec::new();
    let mut actions = Vec::new();
    let mut check = |id: String, ready: bool, blocking: bool, evidence: String| {
        checks.push(ReadinessCheck {
            id,
            ready: Some(ready),
            blocking,
            evidence,
        });
    };
    let accessible = run
        .repository_git
        .as_ref()
        .is_some_and(|owned| runner.bind_git(owned).is_ok());
    let planning_owned = run
        .planning_workspace
        .git
        .as_ref()
        .is_some_and(|owned| runner.bind_git(owned).is_ok());
    check(
        "repository".into(),
        accessible,
        true,
        "Repository accessibility and pinned Git ownership".into(),
    );
    check(
        "planning-workspace".into(),
        planning_owned,
        true,
        "Planning checkout accessibility and pinned Git ownership".into(),
    );
    let clean = if accessible {
        runner
            .git(&run.repository, &["status", "--porcelain"], stop)
            .await
            .is_ok_and(|o| o.is_empty())
    } else {
        false
    };
    check(
        "git-working-state".into(),
        clean,
        false,
        "Developer checkout cleanliness; execution uses the pinned base in isolated worktrees"
            .into(),
    );
    for program in ["codex", "claude"] {
        check(
            format!("installed-{program}"),
            available(program, &run.repository),
            false,
            format!(
                "Optional installed provider executable: {program}; configured adapters are capability-checked separately"
            ),
        );
    }
    for (index, command) in run.settings.verification.commands.iter().enumerate() {
        check(format!("verification-{index}"), available(&command[0], &run.planning_workspace.path), true,
            "Configured verification executable availability; command behavior is validated during execution".into());
    }
    for tool in &run.settings.preflight.required_tools {
        check(
            format!("tool-{tool}"),
            available(tool, &run.repository),
            true,
            format!("Required executable: {tool}"),
        );
    }
    for (index, directory) in run
        .settings
        .preflight
        .required_directories
        .iter()
        .enumerate()
    {
        let path = run.planning_workspace.path.join(directory);
        let valid = std::fs::canonicalize(&path)
            .is_ok_and(|p| p.starts_with(&run.planning_workspace.path) && p.is_dir());
        check(
            format!("directory-{index}"),
            valid,
            true,
            format!("Required repository directory: {}", directory.display()),
        );
    }
    for name in &run.settings.preflight.required_env {
        // Nonempty presence only; providers determine validity. Never persist values.
        let present = std::env::var_os(name).is_some_and(|v| !v.is_empty());
        check(
            format!("env-{name}"),
            present,
            true,
            format!(
                "Required environment variable {name}: {}",
                if present { "present" } else { "missing" }
            ),
        );
        if !present {
            actions.push(action(requirement(HumanActionType::Environment, format!("Environment variable {name} is missing"), format!("Configure {name} through the existing service/login mechanism; restart the daemon if its inherited environment changes. Never submit the value.")), &format!("env-{name}"), false, None));
        }
    }
    for service in &run.settings.preflight.external_services {
        actions.push(action(requirement(HumanActionType::Environment, format!("External prerequisite {service}"), format!("Confirm {service} is configured through its existing mechanism; provide no credentials")), &format!("service-{service}"), false, None));
    }
    let requirements = planner_requirements(
        run.plan
            .as_ref()
            .map(|p| p.human_requirements.as_slice())
            .unwrap_or_default(),
        &run.human_actions,
    );
    for proposed in &requirements {
        let mut proposed = proposed.clone();
        // A model can propose a question or exact operation; it cannot approve it.
        proposed.reason = safe_text(&proposed.reason)?;
        proposed.requested_action = safe_text(&proposed.requested_action)?;
        proposed.evidence = safe_text(&proposed.evidence)?;
        if let Some(permission) = &proposed.permission {
            safe_text(&permission.scope)?;
        }
        actions.push(action(
            proposed.clone(),
            &serde_json::to_string(&proposed)?,
            false,
            None,
        ));
    }
    // Decision actions remain part of refreshed readiness, including task-local holds.
    actions.extend(
        run.human_actions
            .iter()
            .filter(|a| {
                run.decisions.iter().any(|d| {
                    !d.autonomous()
                        && !d.superseded()
                        && a.requirement.evidence == format!("Daemon decision assessment: {}", d.id)
                })
            })
            .cloned(),
    );
    // Baseline harness operations are brokered too; policy cannot silently deny checks/writes.
    for operation in [
        PermissionOperation::WorkspaceWrite,
        PermissionOperation::RunChecks,
    ] {
        let permission = PermissionRequest {
            operation,
            scope: if operation == PermissionOperation::WorkspaceWrite {
                format!("run {} assigned task worktrees", run.id)
            } else {
                format!("run {} immutable configured verification commands", run.id)
            },
        };
        check(
            format!("permission-{operation:?}"),
            run.settings.permissions.classify(operation) == PermissionRule::Allow,
            false,
            format!(
                "{:?}: {:?}; scope {}",
                operation,
                run.settings.permissions.classify(operation),
                permission.scope
            ),
        );
        if run.settings.permissions.classify(operation) != PermissionRule::Allow
            && !super::delegation::covers(&run, &permission)
        {
            let mut req = requirement(
                HumanActionType::Permission,
                "Execution requires a configured permission".into(),
                "Approve or reject this exact run operation".into(),
            );
            req.permission = Some(permission.clone());
            actions.push(action(
                req,
                &serde_json::to_string(&permission)?,
                false,
                None,
            ));
        }
    }
    checks.extend(agents.authentication_readiness(run.delegation.is_some()));
    if accessible && planning_owned {
        checks.extend(
            agents
                .readiness(runner, &run.planning_workspace.path, &run.settings, stop)
                .await?,
        );
    } else {
        checks.push(ReadinessCheck {
            id: "provider-probes".into(),
            ready: None,
            blocking: true,
            evidence: "Provider probes skipped because pinned workspace ownership is unavailable"
                .into(),
        });
    }
    if run.delegation.is_some() {
        checks.extend(super::delegation::readiness(&run, runner, stop).await?);
    }
    // Failed deterministic checks have stable checklist actions too. Resolving an
    // acknowledgement cannot override the failed check; the next probe must pass.
    for c in checks
        .iter()
        .filter(|c| c.blocking && c.ready == Some(false) && !c.id.starts_with("env-"))
    {
        actions.push(action(
            requirement(
                HumanActionType::Environment,
                c.evidence.clone(),
                "Repair this prerequisite externally and rerun preflight; never submit credentials"
                    .into(),
            ),
            &format!("check-{}", c.id),
            false,
            None,
        ));
    }
    runner.check_authority(stop)?;
    let ids: Vec<_> = actions.iter().map(|a| a.id.clone()).collect();
    journal.change("preflight", "Preflight checks and human checklist refreshed (no credential values)", |r| {
        runner.check_authority(stop)?;
        for a in &mut r.human_actions {
            let passed = checks.iter().any(|c| c.ready == Some(true) && (a.id == stable_id(&format!("check-{}", c.id)) || (c.id.starts_with("env-") && a.id == stable_id(&c.id))));
            if passed && a.requirement.kind == HumanActionType::Environment && a.status != HumanActionStatus::Resolved {
                a.status = HumanActionStatus::Resolved;
                a.updated_at = Utc::now();
                a.audit.push(ActionAudit { at: a.updated_at, status: a.status, request: None });
            }
        }
        for candidate in actions {
            if let Some(existing) = r.human_actions.iter_mut().find(|a| a.id == candidate.id) {
                // A failed host check cannot be waived by an earlier acknowledgement.
                let failed_check = checks.iter().any(|c| c.blocking && c.ready != Some(true) && (candidate.id == stable_id(&format!("check-{}", c.id)) || (c.id.starts_with("env-") && candidate.id == stable_id(&c.id))));
                if failed_check && existing.status == HumanActionStatus::Resolved {
                    existing.status = HumanActionStatus::Pending;
                    existing.resolution = None;
                    existing.updated_at = Utc::now();
                    existing.audit.push(ActionAudit { at: existing.updated_at, status: existing.status, request: None });
                }
            } else { r.human_actions.push(candidate); }
        }
        let ready = !checks.iter().any(|c| c.blocking && c.ready != Some(true)) &&
            !r.human_actions.iter().filter(|a| ids.contains(&a.id)).any(|a| r.action_blocks(a));
        r.preflight = Some(PreflightReport { checked_at: Utc::now(), checks, action_ids: ids, ready, independent_work_ready: false,
            limitation: "Best-effort checks of declared prerequisites and material planning decisions; cannot discover or guarantee absence of future human dependencies. Authentication validity may be unknown until invocation.".into() });
        let independent = r.autonomous_preflight_ready();
        if let Some(p) = &mut r.preflight { p.independent_work_ready = independent; }
        Ok(())
    }).await
}

/// Reject likely credential material instead of retaining it in a resolution or new finding.
/// This is an input guard, not a secret manager or a general source-code scrubber.
pub(super) fn safe_text(value: &str) -> Result<String> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 16384 && !value.contains('\0'),
        "provide a bounded, nonempty explanation without secrets"
    );
    let lower = value.to_ascii_lowercase();
    ensure!(
        !lower.trim_start().starts_with("sk-")
            && ![
                "-----begin",
                "bearer ",
                " sk-",
                "password=",
                "token=",
                "secret=",
                "://"
            ]
            .iter()
            .any(|s| lower.contains(s)),
        "credential-like material is forbidden; configure credentials externally"
    );
    Ok(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_new_requirements_group_but_legacy_action_identities_survive_refresh() {
        let mut first = requirement(
            HumanActionType::Environment,
            "Shared account".into(),
            "Configure account externally".into(),
        );
        first.affected_tasks = vec!["B".into(), "A".into()];
        let mut second = first.clone();
        second.affected_tasks = vec!["C".into()];
        let proposed = vec![first.clone(), second.clone()];
        let grouped = planner_requirements(&proposed, &[]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].affected_tasks, vec!["A", "B", "C"]);
        let mut legacy = action(
            first.clone(),
            &serde_json::to_string(&first).expect("requirement"),
            false,
            None,
        );
        legacy.prerequisite = None;
        legacy.status = HumanActionStatus::Resolved;
        let original_id = legacy.id.clone();
        let refreshed = planner_requirements(&proposed, &[legacy]);
        assert_eq!(refreshed, proposed);
        assert_eq!(
            action(
                refreshed[0].clone(),
                &serde_json::to_string(&refreshed[0]).expect("requirement"),
                false,
                None
            )
            .id,
            original_id
        );
        let mut permission = first;
        permission.kind = HumanActionType::Permission;
        permission.permission = Some(PermissionRequest {
            operation: PermissionOperation::ExternalOperation,
            scope: "account one".into(),
        });
        let mut other = permission.clone();
        other.permission.as_mut().expect("permission").scope = "account two".into();
        assert_eq!(planner_requirements(&[permission, other], &[]).len(), 2);
    }
}
