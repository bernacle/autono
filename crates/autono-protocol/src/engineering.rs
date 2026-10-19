//! Engineering workflow contracts. Process runs remain separate execution attempts.
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
            Self::AwaitingApproval => matches!(next, Self::Running | Self::Cancelled),
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
                    Self::Running | Self::Cancelled | Self::Interrupted
                ) | (
                    Self::Running,
                    Self::Completed
                        | Self::Blocked
                        | Self::Failed
                        | Self::Cancelled
                        | Self::Interrupted
                )
            )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub path: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub head_commit: Option<String>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub process_key: Uuid,
    pub process_run_id: Uuid,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub passed: bool,
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
    pub criterion: String,
    pub satisfied: bool,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub accepted: bool,
    pub criteria: Vec<CriterionReview>,
    pub findings: Vec<ReviewFinding>,
    pub blocker: String,
}
impl ReviewResult {
    pub fn validate(&self, task: &PlannedTask) -> Result<()> {
        ensure!(
            self.findings.len() <= 64 && self.criteria.len() == task.acceptance_criteria.len(),
            "review must account for every acceptance criterion"
        );
        let actual: BTreeSet<_> = self.criteria.iter().map(|c| c.criterion.as_str()).collect();
        let expected: BTreeSet<_> = task
            .acceptance_criteria
            .iter()
            .map(String::as_str)
            .collect();
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
            && self.blocker.is_empty()
            && self.criteria.iter().all(|c| c.satisfied)
            && !self.findings.iter().any(ReviewFinding::blocking)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAttempt {
    pub number: usize,
    pub coder: AgentSession,
    pub verification: Vec<VerificationResult>,
    pub reviewer: Option<AgentSession>,
    pub review: Option<ReviewResult>,
    pub snapshot: Option<String>,
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
    pub id: Uuid,
    pub revision: i64,
    pub status: EngineeringStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repository: PathBuf,
    pub requested_base: String,
    pub base_commit: String,
    pub specification: String,
    pub settings: WorkflowSettings,
    pub plan: Option<ExecutionPlan>,
    pub approved_at: Option<DateTime<Utc>>,
    pub planner: Option<AgentSession>,
    pub planning_workspace: Workspace,
    pub tasks: Vec<EngineeringTask>,
    pub blockers: Vec<Blocker>,
    pub integration_workspace: Option<Workspace>,
    pub integration_verification: Vec<VerificationResult>,
    pub summary: String,
}
impl EngineeringRun {
    pub fn ready_tasks(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Ready | TaskStatus::Waiting)
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
