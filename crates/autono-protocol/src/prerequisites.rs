//! Read-only prerequisite/receipt projections. HumanAction owns resolution and authority.
use crate::engineering::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrerequisiteCategory {
    EssentialBeforeStart,
    DeferrableManualDependency,
    SafelyResolvableInternally,
    UnsafeOrUncertain,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerequisiteClassification {
    pub category: PrerequisiteCategory,
    /// Daemon evidence for classification; model confidence is never authority.
    pub basis: String,
    /// Only an accepted A3 record can establish a safe alternative. Never a grant.
    pub alternative_decision_id: Option<String>,
    pub verification_remaining: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerequisiteImpact {
    pub action_id: String,
    pub action_version: usize,
    pub category: Option<PrerequisiteCategory>,
    pub affected_tasks: Vec<String>,
    pub waiting_descendants: Vec<String>,
    pub requested_action: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskDelivery {
    IndependentlyVerifiedLocally,
    ImplementedNotFullyVerified,
    BlockedAwaitingPrerequisite,
    NotImplemented,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHandoff {
    pub task_id: String,
    pub delivery: TaskDelivery,
    pub implementation_recorded: bool,
    pub checks_passed: bool,
    pub artifact_commit: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffOutcome {
    InProgress,
    PartiallyCompleted,
    Blocked,
    Failed,
    VerifiedLocally,
    EvidenceIncomplete,
    Completed,
    Cancelled,
    Interrupted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunHandoff {
    pub outcome: HandoffOutcome,
    pub summary: String,
    pub tasks: Vec<TaskHandoff>,
    pub needs_you: Vec<PrerequisiteImpact>,
    /// Category C uses the existing ledger, never a parallel human action/grant.
    pub internal_decision_ids: Vec<String>,
    pub combined_checks_passed: bool,
    pub completion_requirements_satisfied: bool,
}
impl EngineeringRun {
    pub fn prerequisite_impact(&self, action: &HumanAction) -> PrerequisiteImpact {
        let affected: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| {
                action.requirement.affected_tasks.is_empty()
                    || action.requirement.affected_tasks.contains(&t.plan.id)
            })
            .map(|t| t.plan.id.clone())
            .collect();
        let mut descendants = affected.clone();
        loop {
            let old = descendants.len();
            for task in &self.tasks {
                if !descendants.contains(&task.plan.id)
                    && task
                        .plan
                        .dependencies
                        .iter()
                        .any(|id| descendants.contains(id))
                {
                    descendants.push(task.plan.id.clone());
                }
            }
            if old == descendants.len() {
                break;
            }
        }
        PrerequisiteImpact {
            action_id: action.id.clone(),
            action_version: action.audit.len(),
            category: action.prerequisite.as_ref().map(|p| p.category),
            waiting_descendants: descendants
                .into_iter()
                .filter(|id| {
                    !affected.contains(id)
                        && self
                            .tasks
                            .iter()
                            .any(|t| &t.plan.id == id && t.status != TaskStatus::Completed)
                })
                .collect(),
            affected_tasks: affected,
            requested_action: action.requirement.requested_action.clone(),
        }
    }
    /// A truthful projection, never an alternative success state or authority source.
    pub fn partial_completion(&self) -> RunHandoff {
        let needs_you: Vec<_> = self
            .human_actions
            .iter()
            .filter(|a| self.action_blocks(a))
            .map(|a| self.prerequisite_impact(a))
            .collect();
        let tasks: Vec<_> = self
            .tasks
            .iter()
            .map(|t| {
                let last = t.attempts.last();
                let implementation_recorded = t.artifact_commit.is_some()
                    || last.is_some_and(|a| {
                        a.implementation_tree.is_some() || a.verified_tree.is_some()
                    });
                let checks_passed = last.is_some_and(|a| {
                    complete_checks(&a.verification, &self.settings.verification.commands)
                });
                let verified = t.status == TaskStatus::Completed
                    && t.artifact_commit.is_some()
                    && last.is_some_and(|a| {
                        a.verified_tree.is_some()
                            && a.verified_tree == a.preserved_tree
                            && a.review.as_ref().is_some_and(ReviewResult::accepts)
                    })
                    && checks_passed;
                let blocked = t.status == TaskStatus::BlockedHuman
                    || needs_you.iter().any(|p| {
                        p.affected_tasks.contains(&t.plan.id)
                            || p.waiting_descendants.contains(&t.plan.id)
                    });
                TaskHandoff {
                    task_id: t.plan.id.clone(),
                    implementation_recorded,
                    checks_passed,
                    artifact_commit: t.artifact_commit.clone(),
                    delivery: if verified {
                        TaskDelivery::IndependentlyVerifiedLocally
                    } else if matches!(t.status, TaskStatus::Failed | TaskStatus::Blocked) {
                        TaskDelivery::Failed
                    } else if implementation_recorded {
                        TaskDelivery::ImplementedNotFullyVerified
                    } else if blocked {
                        TaskDelivery::BlockedAwaitingPrerequisite
                    } else {
                        TaskDelivery::NotImplemented
                    },
                }
            })
            .collect();
        let verified = tasks
            .iter()
            .filter(|t| t.delivery == TaskDelivery::IndependentlyVerifiedLocally)
            .count();
        let combined_checks_passed = complete_checks(
            &self.integration_verification,
            &self.settings.verification.commands,
        );
        let completion_requirements_satisfied = !tasks.is_empty()
            && verified == tasks.len()
            && needs_you.is_empty()
            && self.status == EngineeringStatus::Completed
            && combined_checks_passed;
        let outcome = match self.status {
            EngineeringStatus::Completed if completion_requirements_satisfied => {
                HandoffOutcome::Completed
            }
            EngineeringStatus::Verified
                if !tasks.is_empty() && verified == tasks.len() && needs_you.is_empty() =>
            {
                HandoffOutcome::VerifiedLocally
            }
            EngineeringStatus::Verified | EngineeringStatus::Completed => {
                HandoffOutcome::EvidenceIncomplete
            }
            EngineeringStatus::Failed => HandoffOutcome::Failed,
            EngineeringStatus::Cancelled => HandoffOutcome::Cancelled,
            EngineeringStatus::Interrupted => HandoffOutcome::Interrupted,
            EngineeringStatus::Blocked if verified > 0 => HandoffOutcome::PartiallyCompleted,
            EngineeringStatus::Blocked | EngineeringStatus::AwaitingApproval
                if !needs_you.is_empty() =>
            {
                HandoffOutcome::Blocked
            }
            _ => HandoffOutcome::InProgress,
        };
        let summary = format!(
            "{verified} of {} tasks implemented and independently verified locally. {} unresolved prerequisites. Combined integration verification {}. {}",
            tasks.len(),
            needs_you.len(),
            if combined_checks_passed {
                "passed"
            } else {
                "not passed"
            },
            if completion_requirements_satisfied {
                "Original completion gates satisfied."
            } else {
                "Full feature completion is not established; external service success is not implied by local checks."
            }
        );
        RunHandoff {
            outcome,
            summary,
            tasks,
            needs_you,
            combined_checks_passed,
            completion_requirements_satisfied,
            internal_decision_ids: self
                .decisions
                .iter()
                .filter(|d| d.autonomous() && !d.superseded())
                .map(|d| d.id.clone())
                .collect(),
        }
    }
}

fn complete_checks(checks: &[VerificationResult], commands: &[Vec<String>]) -> bool {
    !commands.is_empty()
        && checks.len() == commands.len()
        && checks.iter().zip(commands).all(|(check, command)| {
            check.passed
                && check.exit_code == Some(0)
                && check.output_complete
                && &check.command == command
                && check
                    .outcome
                    .is_none_or(|o| o == VerificationOutcome::Passed)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_passing_prefix_or_different_contract_is_not_full_verification() {
        let commands = vec![vec!["first".into()], vec!["second".into()]];
        let check = |command: Vec<String>| VerificationResult {
            process_key: uuid::Uuid::new_v4(),
            process_run_id: uuid::Uuid::new_v4(),
            command,
            exit_code: Some(0),
            passed: true,
            outcome: Some(VerificationOutcome::Passed),
            output_complete: true,
            summary: "actual receipt".into(),
        };
        let mut checks = vec![check(commands[0].clone())];
        assert!(!complete_checks(&checks, &commands));
        checks.push(check(commands[1].clone()));
        assert!(complete_checks(&checks, &commands));
        checks.swap(0, 1);
        assert!(!complete_checks(&checks, &commands));
        checks.swap(0, 1);
        checks[1].output_complete = false;
        assert!(!complete_checks(&checks, &commands));
        assert!(!complete_checks(&[], &[]));
    }
}
