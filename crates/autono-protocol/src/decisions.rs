//! Decision proposals are data, never permission requests or executable operations.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    LocalPattern,
    InternalName,
    DefaultValue,
    TestStructure,
    Significant,
    Authorization,
    UnknownEffects,
    Prerequisite,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionCategory {
    Reversible,
    Significant,
    Authorization,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionEvidence {
    /// None refers to the original developer specification, never an arbitrary URL.
    pub file: Option<String>,
    pub excerpt: String,
    /// The option this excerpt supports. Alternatives cannot silently become support.
    pub supports: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionProposal {
    /// Stable ambiguity key. A different choice must explicitly supersede its predecessor.
    pub key: String,
    pub question: String,
    pub chosen: String,
    pub alternatives: Vec<String>,
    pub rationale: String,
    pub kind: DecisionKind,
    pub evidence: Vec<DecisionEvidence>,
    /// Exact original specification excerpts, checked by the daemon.
    pub requirements: Vec<String>,
    pub assumptions: Vec<String>,
    pub uncertainty: Vec<String>,
    pub affected_files: Vec<String>,
    pub affected_tasks: Vec<String>,
    pub supersedes: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionProvenance {
    pub file: Option<String>,
    pub content_sha256: String,
    pub excerpt: String,
    pub supports: String,
    pub base_commit: String,
    pub workspace_id: Uuid,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEventKind {
    Accepted,
    Unresolved,
    Implemented,
    VerificationFailed,
    ChecksPassed,
    ReviewAccepted,
    ReviewRejected,
    TaskVerified,
    Superseded,
    HumanFeedback,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionEvent {
    pub sequence: usize,
    pub at: DateTime<Utc>,
    pub kind: DecisionEventKind,
    pub task_id: Option<String>,
    pub attempt: Option<usize>,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRecord {
    pub id: String,
    pub run_id: Uuid,
    pub ordinal: usize,
    pub task_id: Option<String>,
    pub attempt: Option<usize>,
    pub session_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub proposal: DecisionProposal,
    pub category: DecisionCategory,
    /// Harness assessment, not the model's self-classification.
    pub policy_basis: String,
    /// Only excerpts checked against actual authorized bytes appear here.
    pub evidence: Vec<DecisionProvenance>,
    pub audit: Vec<DecisionEvent>,
}
impl DecisionRecord {
    pub fn autonomous(&self) -> bool {
        self.audit
            .first()
            .is_some_and(|e| e.kind == DecisionEventKind::Accepted)
    }
    pub fn superseded(&self) -> bool {
        self.audit
            .iter()
            .any(|e| e.kind == DecisionEventKind::Superseded)
    }
    /// All affected tasks need preserved, independently reviewed artifacts.
    pub fn verified(&self) -> bool {
        self.autonomous()
            && !self.superseded()
            && self.proposal.affected_tasks.iter().all(|task| {
                self.audit
                    .iter()
                    .rev()
                    .find(|e| e.task_id.as_ref() == Some(task))
                    .is_some_and(|e| e.kind == DecisionEventKind::TaskVerified)
            })
    }
}
