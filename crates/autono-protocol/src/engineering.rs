//! Engineering workflow contracts. Process runs remain separate execution attempts.
pub use crate::decisions::*;
pub use crate::prerequisites::*;
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Claude,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Planner,
    Coder,
    Reviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub provider: Provider,
    pub model: Option<String>,
    pub timeout_seconds: u64,
}
impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            provider: Provider::Claude,
            model: None,
            timeout_seconds: 600,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    pub planner: AgentConfig,
    pub coder: AgentConfig,
    pub reviewer: AgentConfig,
}
impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            planner: AgentConfig::default(),
            coder: AgentConfig {
                provider: Provider::Codex,
                ..AgentConfig::default()
            },
            reviewer: AgentConfig::default(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionConfig {
    pub max_parallel: usize,
    pub max_attempts: usize,
    pub max_review_cycles: usize,
}
impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_parallel: 2,
            max_attempts: 3,
            max_review_cycles: 2,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerificationConfig {
    /// Argument arrays; no implicit shell. Commands are trusted developer configuration.
    pub commands: Vec<Vec<String>>,
    pub timeout_seconds: u64,
}
impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            commands: vec![
                vec!["cargo".into(), "fmt".into(), "--check".into()],
                vec!["cargo".into(), "test".into()],
            ],
            timeout_seconds: 300,
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSettings {
    pub agents: AgentsConfig,
    pub execution: ExecutionConfig,
    pub verification: VerificationConfig,
    #[serde(default)]
    pub preflight: PreflightConfig,
    #[serde(default)]
    pub permissions: PermissionPolicy,
}
impl WorkflowSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8).contains(&self.execution.max_parallel),
            "max_parallel must be 1..8"
        );
        ensure!(
            (1..=5).contains(&self.execution.max_attempts),
            "max_attempts must be 1..5"
        );
        ensure!(
            (1..=5).contains(&self.execution.max_review_cycles),
            "max_review_cycles must be 1..5"
        );
        for agent in [
            &self.agents.planner,
            &self.agents.coder,
            &self.agents.reviewer,
        ] {
            ensure!(
                (1..=3600).contains(&agent.timeout_seconds),
                "agent timeout must be 1..3600 seconds"
            );
            ensure!(
                agent
                    .model
                    .as_ref()
                    .is_none_or(|s| !s.trim().is_empty() && s.len() <= 200),
                "invalid model"
            );
        }
        ensure!(
            (1..=16).contains(&self.verification.commands.len()),
            "configure 1..16 verification commands"
        );
        ensure!(
            (1..=3600).contains(&self.verification.timeout_seconds),
            "verification timeout must be 1..3600"
        );
        for command in &self.verification.commands {
            ensure!(
                !command.is_empty() && !command[0].is_empty() && command.len() <= 64,
                "invalid verification argument array"
            );
            ensure!(
                command.iter().all(|s| !s.contains('\0') && s.len() <= 4096),
                "invalid verification argument"
            );
        }
        self.preflight.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub id: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Milestone {
    pub id: String,
    pub goal: String,
    pub task_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedTask {
    pub id: String,
    pub goal: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub dependencies: Vec<String>,
    pub requirement_ids: Vec<String>,
    pub verification_expectations: Vec<String>,
    pub role: AgentRole,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionProposal>,
    #[serde(default)]
    pub human_requirements: Vec<HumanRequirement>,
    pub requirements: Vec<Requirement>,
    pub milestones: Vec<Milestone>,
    pub tasks: Vec<PlannedTask>,
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 16_384
}
impl ExecutionPlan {
    /// A deterministic topological order, also used for local integration.
    pub fn validate(&self) -> Result<Vec<String>> {
        ensure!(
            (1..=32).contains(&self.tasks.len()),
            "plan must contain 1..32 tasks"
        );
        ensure!(
            (1..=64).contains(&self.requirements.len()),
            "plan must contain 1..64 requirements"
        );
        let requirements: BTreeSet<_> = self.requirements.iter().map(|r| r.id.as_str()).collect();
        ensure!(
            requirements.len() == self.requirements.len()
                && self
                    .requirements
                    .iter()
                    .all(|r| identifier(&r.id) && text(&r.text)),
            "invalid or duplicate requirements"
        );
        let tasks: BTreeMap<_, _> = self.tasks.iter().map(|t| (t.id.as_str(), t)).collect();
        ensure!(tasks.len() == self.tasks.len(), "duplicate task ID");
        let mut covered = BTreeSet::new();
        for task in &self.tasks {
            ensure!(
                identifier(&task.id) && text(&task.goal) && text(&task.description),
                "invalid task text or ID"
            );
            ensure!(
                task.role == AgentRole::Coder,
                "implementation tasks require coder role"
            );
            ensure!(
                (1..=16).contains(&task.acceptance_criteria.len())
                    && task.acceptance_criteria.iter().all(|s| text(s)),
                "task needs 1..16 acceptance criteria"
            );
            ensure!(
                task.acceptance_criteria
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == task.acceptance_criteria.len(),
                "duplicate acceptance criterion"
            );
            ensure!(
                !task.verification_expectations.is_empty()
                    && task.verification_expectations.len() <= 16
                    && task.verification_expectations.iter().all(|s| text(s)),
                "missing verification expectations"
            );
            ensure!(
                task.dependencies.len() <= 32
                    && task.dependencies.iter().collect::<BTreeSet<_>>().len()
                        == task.dependencies.len(),
                "duplicate dependency"
            );
            ensure!(
                task.dependencies
                    .iter()
                    .all(|id| id != &task.id && tasks.contains_key(id.as_str())),
                "unknown or self dependency"
            );
            ensure!(
                !task.requirement_ids.is_empty()
                    && task.requirement_ids.len() <= 64
                    && task
                        .requirement_ids
                        .iter()
                        .all(|id| requirements.contains(id.as_str())),
                "unknown or missing requirement coverage"
            );
            covered.extend(task.requirement_ids.iter().map(String::as_str));
        }
        ensure!(covered == requirements, "plan omits requirements");
        ensure!(
            (1..=8).contains(&self.milestones.len()),
            "plan needs 1..8 milestones"
        );
        let mut assigned = BTreeSet::new();
        let mut milestone_ids = BTreeSet::new();
        for m in &self.milestones {
            ensure!(
                identifier(&m.id)
                    && milestone_ids.insert(&m.id)
                    && text(&m.goal)
                    && !m.task_ids.is_empty(),
                "invalid milestone"
            );
            for id in &m.task_ids {
                ensure!(
                    tasks.contains_key(id.as_str()) && assigned.insert(id.as_str()),
                    "unknown or multiply assigned milestone task"
                );
            }
        }
        ensure!(assigned.len() == tasks.len(), "milestones omit tasks");
        let mut done = BTreeSet::new();
        let mut order = Vec::new();
        while order.len() < tasks.len() {
            let next = tasks.values().find(|t| {
                !done.contains(t.id.as_str())
                    && t.dependencies.iter().all(|d| done.contains(d.as_str()))
            });
            let Some(next) = next else {
                anyhow::bail!("dependency cycle");
            };
            done.insert(next.id.as_str());
            order.push(next.id.clone());
        }
        for requirement in &self.human_requirements {
            ensure!(
                requirement.kind != HumanActionType::Runtime,
                "planner cannot create runtime authority"
            );
            requirement.validate(&self.tasks)?;
        }
        ensure!(
            self.human_requirements.len() <= 64,
            "too many human requirements"
        );
        Ok(order)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EngineeringStatus {
    Planning,
    AwaitingApproval,
    Running,
    Verified,
    Integrating,
    Completed,
    Blocked,
    Failed,
    Cancelled,
    Interrupted,
}
impl EngineeringStatus {
    pub fn active(self) -> bool {
        matches!(self, Self::Planning | Self::Running | Self::Integrating)
    }
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Planning => matches!(
                next,
                Self::AwaitingApproval
                    | Self::Blocked
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Interrupted
            ),
            Self::Blocked => matches!(next, Self::Running | Self::Cancelled),
            Self::AwaitingApproval => {
                matches!(next, Self::Planning | Self::Running | Self::Cancelled)
            }
            Self::Running => matches!(
                next,
                Self::Verified | Self::Blocked | Self::Failed | Self::Cancelled | Self::Interrupted
            ),
            Self::Verified => matches!(next, Self::Integrating | Self::Cancelled),
            Self::Integrating => matches!(
                next,
                Self::Completed
                    | Self::Blocked
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Interrupted
            ),
            _ => false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskStatus {
    BlockedHuman,
    Waiting,
    Ready,
    Running,
    Blocked,
    Failed,
    Completed,
    Cancelled,
    Interrupted,
}
impl TaskStatus {
    pub fn can_transition(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (
                    Self::Waiting,
                    Self::Ready | Self::Blocked | Self::Cancelled | Self::Interrupted
                ) | (
                    Self::Ready,
                    Self::BlockedHuman | Self::Running | Self::Cancelled | Self::Interrupted
                ) | (
                    Self::Running,
                    Self::BlockedHuman
                        | Self::Completed
                        | Self::Blocked
                        | Self::Failed
                        | Self::Cancelled
                        | Self::Interrupted
                ) | (
                    Self::BlockedHuman,
                    Self::Ready | Self::Cancelled | Self::Interrupted
                )
            )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitOwnership {
    pub worktree: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    /// SHA-256 of ownership/configuration files; None records an absent file.
    pub metadata: BTreeMap<PathBuf, Option<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub path: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub head_commit: Option<String>,
    #[serde(default)]
    pub git: Option<GitOwnership>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    pub task_id: Option<String>,
    pub kind: String,
    pub detail: String,
    pub action: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: Uuid,
    pub role: AgentRole,
    pub provider: Provider,
    pub provider_session_id: Option<String>,
    pub resumed_from: Option<String>,
    pub process_key: Uuid,
    pub process_run_id: Option<Uuid>,
    pub outcome: Option<String>,
    pub result: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Passed,
    TestFailure,
    Timeout,
    Cancelled,
    InfrastructureFailure,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub process_key: Uuid,
    pub process_run_id: Uuid,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub passed: bool,
    #[serde(default)]
    pub outcome: Option<VerificationOutcome>,
    pub output_complete: bool,
    pub summary: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewFinding {
    pub severity: Severity,
    pub file: String,
    pub line: u32,
    pub explanation: String,
    pub recommended_fix: String,
    pub confidence: f64,
}
impl ReviewFinding {
    pub fn blocking(&self) -> bool {
        matches!(
            self.severity,
            Severity::Critical | Severity::High | Severity::Medium
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionReview {
    #[serde(default)]
    pub criterion_id: String,
    pub criterion: String,
    pub satisfied: bool,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_reviews: Vec<DecisionReview>,
    pub accepted: bool,
    pub criteria: Vec<CriterionReview>,
    pub findings: Vec<ReviewFinding>,
    pub blocker: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionReview {
    pub decision_id: String,
    pub implemented: bool,
    pub evidence: String,
}
impl ReviewResult {
    pub fn validate_decisions(&self, ids: &[String]) -> Result<()> {
        let actual: BTreeSet<_> = self
            .decision_reviews
            .iter()
            .map(|d| &d.decision_id)
            .collect();
        ensure!(
            actual.len() == self.decision_reviews.len()
                && actual == ids.iter().collect()
                && self
                    .decision_reviews
                    .iter()
                    .all(|d| !d.evidence.trim().is_empty() && d.evidence.len() <= 4000),
            "review must independently account for the exact consumed decision revisions"
        );
        Ok(())
    }
    pub fn validate(&self, task: &PlannedTask) -> Result<()> {
        ensure!(
            self.findings.len() <= 64 && self.criteria.len() == task.acceptance_criteria.len(),
            "review must account for every acceptance criterion"
        );
        // Old persisted reviews used exact labels. New responses identify immutable
        // criteria by task ID and ordinal, so paraphrasing a label is harmless.
        let legacy = self.criteria.iter().all(|c| c.criterion_id.is_empty());
        let actual: BTreeSet<_> = self
            .criteria
            .iter()
            .map(|c| {
                if legacy {
                    c.criterion.clone()
                } else {
                    c.criterion_id.clone()
                }
            })
            .collect();
        let expected: BTreeSet<_> = if legacy {
            task.acceptance_criteria.iter().cloned().collect()
        } else {
            task.criterion_ids().into_iter().collect()
        };
        ensure!(
            actual == expected && self.criteria.iter().all(|c| text(&c.evidence)),
            "invalid acceptance evidence"
        );
        ensure!(
            self.findings.iter().all(|f| f.confidence.is_finite()
                && (0.0..=1.0).contains(&f.confidence)
                && text(&f.explanation)
                && text(&f.recommended_fix)
                && f.file.len() <= 4096),
            "invalid review finding"
        );
        Ok(())
    }
    pub fn accepts(&self) -> bool {
        self.accepted
            && self.decision_reviews.iter().all(|d| d.implemented)
            && self.blocker.is_empty()
            && self.criteria.iter().all(|c| c.satisfied)
            && !self.findings.iter().any(ReviewFinding::blocking)
    }
}
impl PlannedTask {
    pub fn criterion_ids(&self) -> Vec<String> {
        (1..=self.acceptance_criteria.len())
            .map(|n| format!("{}-C{n}", self.id))
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAttempt {
    pub session: AgentSession,
    pub workspace: Option<Workspace>,
    pub review: Option<ReviewResult>,
    pub diagnostic: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAttempt {
    pub number: usize,
    /// Active revisions supplied to this coder invocation. Frozen at attempt admission.
    #[serde(default)]
    pub decision_context: Vec<String>,
    /// Revisions consumed after validating this response, before applying its edits.
    /// None denotes an unadmitted response or a legacy attempt, never verified attribution.
    #[serde(default)]
    pub decision_ids: Option<Vec<String>>,
    pub coder: AgentSession,
    pub verification: Vec<VerificationResult>,
    pub reviewer: Option<AgentSession>,
    #[serde(default)]
    pub reviews: Vec<ReviewAttempt>,
    pub review: Option<ReviewResult>,
    pub snapshot: Option<String>,
    #[serde(default)]
    pub implementation_tree: Option<String>,
    /// Immutable tree audited against actual bytes/modes before verification.
    #[serde(default)]
    pub verified_tree: Option<String>,
    /// Tree resolved from the preserved commit, checked against verified_tree.
    #[serde(default)]
    pub preserved_tree: Option<String>,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineeringTask {
    pub plan: PlannedTask,
    pub status: TaskStatus,
    pub workspace: Option<Workspace>,
    pub attempts: Vec<TaskAttempt>,
    pub artifact_commit: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineeringRun {
    #[serde(default)]
    pub decisions: Vec<DecisionRecord>,
    #[serde(default)]
    pub handoff: Option<RunHandoff>,
    pub id: Uuid,
    pub revision: i64,
    pub status: EngineeringStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repository: PathBuf,
    #[serde(default)]
    pub repository_git: Option<GitOwnership>,
    pub requested_base: String,
    pub base_commit: String,
    pub specification: String,
    pub settings: WorkflowSettings,
    /// Absent on all legacy and supervised runs. Never inferred from preferences.
    #[serde(default)]
    pub delegation: Option<DelegationAuthorization>,
    pub plan: Option<ExecutionPlan>,
    pub approved_at: Option<DateTime<Utc>>,
    pub planner: Option<AgentSession>,
    pub planning_workspace: Workspace,
    pub tasks: Vec<EngineeringTask>,
    pub blockers: Vec<Blocker>,
    #[serde(default)]
    pub human_actions: Vec<HumanAction>,
    #[serde(default)]
    pub preflight: Option<PreflightReport>,
    pub integration_workspace: Option<Workspace>,
    pub integration_verification: Vec<VerificationResult>,
    pub summary: String,
}
impl EngineeringRun {
    /// A proposal/resolution cannot grant an unsupported permission. Delegation
    /// covers only the two baseline scopes; actual execution revalidates context.
    pub fn action_blocks(&self, action: &HumanAction) -> bool {
        if let Some(p) = &action.requirement.permission {
            return matches!(
                action.status,
                HumanActionStatus::Rejected | HumanActionStatus::Deferred
            ) || self.settings.permissions.classify(p.operation) == PermissionRule::Deny
                || !p.supported(self.id)
                || (self.settings.permissions.classify(p.operation) != PermissionRule::Allow
                    && self.delegation.is_none()
                    && action.status != HumanActionStatus::Resolved);
        }
        action.blocks()
    }
    /// Planning is read-only: execution-only broker approvals do not gate it.
    pub fn essential_prerequisites_ready(&self) -> bool {
        self.preflight
            .as_ref()
            .is_some_and(|p| !p.checks.iter().any(|c| c.blocking && c.ready != Some(true)))
            && !self.human_actions.iter().any(|a| {
                a.requirement.affected_tasks.is_empty()
                    && a.requirement.permission.is_none()
                    && self.action_blocks(a)
            })
    }
    /// All deterministic host gates and run-wide human gates must pass. A scoped
    /// human action blocks its tasks/descendants while independent work may run.
    pub fn autonomous_preflight_ready(&self) -> bool {
        self.delegation.is_some()
            && self
                .preflight
                .as_ref()
                .is_some_and(|p| !p.checks.iter().any(|c| c.blocking && c.ready != Some(true)))
            && !self
                .human_actions
                .iter()
                .any(|a| a.requirement.affected_tasks.is_empty() && self.action_blocks(a))
    }
    pub fn ready_tasks(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Ready | TaskStatus::Waiting)
                    && (self.delegation.is_none()
                        || !self.human_actions.iter().any(|a| {
                            self.action_blocks(a)
                                && (a.requirement.affected_tasks.is_empty()
                                    || a.requirement.affected_tasks.contains(&t.plan.id))
                        }))
                    && t.plan.dependencies.iter().all(|d| {
                        self.tasks
                            .iter()
                            .any(|p| &p.plan.id == d && p.status == TaskStatus::Completed)
                    })
            })
            .map(|t| t.plan.id.clone())
            .collect()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSubmission {
    pub id: Uuid,
    pub repository: PathBuf,
    pub base: String,
    pub specification: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineeringEvent {
    pub sequence: i64,
    pub run_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    pub detail: String,
}

/// A small policy vocabulary; approvals never change provider sandbox flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOperation {
    WorkspaceWrite,
    RunChecks,
    OutsideWorkspace,
    ProtectedConfiguration,
    InstallDependencies,
    EnvironmentChange,
    DestructiveMigration,
    ExternalOperation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PermissionRule {
    Allow,
    RequireApproval,
    Deny,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionPolicy {
    pub rules: BTreeMap<PermissionOperation, PermissionRule>,
}
impl PermissionPolicy {
    pub fn classify(&self, operation: PermissionOperation) -> PermissionRule {
        self.rules
            .get(&operation)
            .copied()
            .unwrap_or(match operation {
                PermissionOperation::WorkspaceWrite | PermissionOperation::RunChecks => {
                    PermissionRule::Allow
                }
                PermissionOperation::OutsideWorkspace
                | PermissionOperation::DestructiveMigration
                | PermissionOperation::ExternalOperation => PermissionRule::Deny,
                _ => PermissionRule::RequireApproval,
            })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRequest {
    pub operation: PermissionOperation,
    /// Exact operation and target. Never credentials or arbitrary shell commands.
    pub scope: String,
}
impl PermissionRequest {
    pub fn supported(&self, run_id: Uuid) -> bool {
        let expected = match self.operation {
            PermissionOperation::WorkspaceWrite => format!("run {run_id} assigned task worktrees"),
            PermissionOperation::RunChecks => {
                format!("run {run_id} immutable configured verification commands")
            }
            _ => return false,
        };
        self.scope == expected
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanActionType {
    Environment,
    ProductDecision,
    Permission,
    Runtime,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanRequirement {
    pub kind: HumanActionType,
    pub reason: String,
    pub requested_action: String,
    pub affected_tasks: Vec<String>,
    pub blocking: bool,
    pub evidence: String,
    pub confidence: u8,
    pub permission: Option<PermissionRequest>,
}
impl HumanRequirement {
    fn validate(&self, tasks: &[PlannedTask]) -> Result<()> {
        ensure!(
            text(&self.reason)
                && text(&self.requested_action)
                && text(&self.evidence)
                && self.confidence <= 100,
            "invalid human requirement"
        );
        ensure!(
            self.affected_tasks.len() <= 32
                && self
                    .affected_tasks
                    .iter()
                    .all(|id| tasks.iter().any(|t| &t.id == id)),
            "unknown affected task"
        );
        ensure!(
            (self.kind == HumanActionType::Permission) == self.permission.is_some(),
            "permission requires exact operation and scope"
        );
        if let Some(permission) = &self.permission {
            ensure!(text(&permission.scope), "missing permission scope");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PreflightConfig {
    pub required_env: Vec<String>,
    pub required_tools: Vec<String>,
    /// Relative directories under the repository, checked without executing code.
    pub required_directories: Vec<PathBuf>,
    /// Human-confirmed prerequisites. Names only, never URLs containing credentials.
    pub external_services: Vec<String>,
}
impl PreflightConfig {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.required_env.len() <= 64
                && self.required_env.iter().all(|s| identifier(s)
                    && !s.contains('-')
                    && !s.as_bytes()[0].is_ascii_digit()),
            "invalid environment variable name"
        );
        ensure!(
            self.required_tools.len() <= 64 && self.required_tools.iter().all(|s| identifier(s)),
            "invalid required tool"
        );
        ensure!(
            self.required_directories.len() <= 64
                && self
                    .required_directories
                    .iter()
                    .all(|p| !p.as_os_str().is_empty()
                        && p.components()
                            .all(|c| matches!(c, std::path::Component::Normal(_)))),
            "required directories must be repository-relative"
        );
        ensure!(
            self.external_services.len() <= 64
                && self.external_services.iter().all(|s| identifier(s)),
            "external services require simple names"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HumanActionStatus {
    Pending,
    Resolved,
    Deferred,
    Rejected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    NotPerformed,
    AlreadyPerformed,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionDisposition {
    Resolve,
    Approve,
    Reject,
    Defer,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRequest {
    /// Optional for legacy clients. New clients bind the observed action audit version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_action_version: Option<usize>,
    pub request_id: Uuid,
    pub disposition: ResolutionDisposition,
    /// Product decisions or evidence of external setup. Never secret material.
    pub note: String,
    pub effect: EffectState,
    pub permission: Option<PermissionRequest>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionAudit {
    pub at: DateTime<Utc>,
    pub status: HumanActionStatus,
    /// None identifies a deterministic system check, never an approval.
    pub request: Option<ResolutionRequest>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanAction {
    /// Absent on legacy records; never inferred as authority.
    #[serde(default)]
    pub prerequisite: Option<PrerequisiteClassification>,
    pub id: String,
    pub requirement: HumanRequirement,
    pub status: HumanActionStatus,
    pub resolution: Option<ResolutionRequest>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub audit: Vec<ActionAudit>,
    /// Safe live pause only. Security/integrity failures cannot be continued.
    pub continuable: bool,
    pub workspace_snapshot: Option<String>,
}
impl HumanAction {
    pub fn blocks(&self) -> bool {
        self.requirement.blocking && self.status != HumanActionStatus::Resolved
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessCheck {
    pub id: String,
    pub ready: Option<bool>,
    pub blocking: bool,
    pub evidence: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReport {
    pub checked_at: DateTime<Utc>,
    pub checks: Vec<ReadinessCheck>,
    pub action_ids: Vec<String>,
    pub ready: bool,
    /// All essential gates pass, but task-scoped requirements may still be pending.
    #[serde(default)]
    pub independent_work_ready: bool,
    pub limitation: String,
}

/// A proposal is inspectable consent material, not permission to start planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationProposalRequest {
    pub submission: PlanSubmission,
    /// v2 only. Absent means the original v1 exact-file authority, unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_scope: Option<SourceScope>,
    /// Exact relative UTF-8 regular files; no globs, deletes, links or mode changes.
    pub writable_files: Vec<String>,
    pub max_runtime_seconds: u64,
    pub local_integration: bool,
}
/// Finite source discovery; never a general filesystem or command capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceScope {
    pub version: u32,
    pub read_subtrees: Vec<String>,
    pub write_subtrees: Vec<String>,
    pub allow_create: bool,
}
impl Default for SourceScope {
    fn default() -> Self {
        Self {
            version: 1,
            read_subtrees: vec!["src".into(), "tests".into()],
            write_subtrees: vec!["src".into(), "tests".into()],
            allow_create: true,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationContract {
    pub version: u32,
    pub request: DelegationProposalRequest,
    pub repository_git: GitOwnership,
    /// Filesystem device/inode/owner identities, in addition to Git metadata hashes.
    pub repository_identity: BTreeMap<PathBuf, String>,
    pub base_commit: String,
    pub worktree_root: PathBuf,
    pub settings: WorkflowSettings,
    pub verification_sandbox: String,
    pub user_config_digest: Option<String>,
    pub limitations: Vec<String>,
    /// Trusted read-only discovery at the pinned base. Informational, not an allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_inventory: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationAuthorization {
    pub contract: DelegationContract,
    pub authorized_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
