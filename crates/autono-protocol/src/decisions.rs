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
    #[serde(default)]
    pub source: DecisionEvidenceSource,
    pub file: Option<String>,
    pub content_sha256: String,
    pub excerpt: String,
    pub supports: String,
    pub base_commit: Option<String>,
    pub workspace_id: Option<Uuid>,
    #[serde(default)]
    pub blob_oid: Option<String>,
}
/// Legacy records retain their bytes without retroactively certifying their origin.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEvidenceSource {
    #[default]
    LegacyUnverified,
    Specification,
    FrozenBase,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEventKind {
    Accepted,
    Unresolved,
    Consumed,
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
                let Some(last) = self
                    .audit
                    .iter()
                    .rev()
                    .find(|e| e.task_id.as_ref() == Some(task))
                else {
                    return false;
                };
                if last.kind != DecisionEventKind::TaskVerified || last.attempt.is_none() {
                    return false;
                }
                let mut sequence = 0;
                [
                    DecisionEventKind::Consumed,
                    DecisionEventKind::Implemented,
                    DecisionEventKind::ChecksPassed,
                    DecisionEventKind::ReviewAccepted,
                    DecisionEventKind::TaskVerified,
                ]
                .iter()
                .all(|kind| {
                    if let Some(e) = self.audit.iter().find(|e| {
                        &e.kind == kind
                            && e.task_id == last.task_id
                            && e.attempt == last.attempt
                            && e.sequence > sequence
                            && e.sequence <= last.sequence
                            && (*kind == DecisionEventKind::Consumed || e.detail == last.detail)
                    }) {
                        sequence = e.sequence;
                        true
                    } else {
                        false
                    }
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn record() -> DecisionRecord {
        serde_json::from_value(json!({"id":"D-fixture","run_id":Uuid::nil(),"ordinal":1,"task_id":null,"attempt":null,"session_id":Uuid::nil(),"created_at":"2026-10-22T00:00:00Z","proposal":{"key":"name","question":"Which helper?","chosen":"stable","alternatives":["legacy"],"rationale":"Reuse naming","kind":"internal_name","evidence":[],"requirements":["Implement"],"assumptions":[],"uncertainty":[],"affected_files":["b.txt"],"affected_tasks":["b"],"supersedes":null},"category":"reversible","policy_basis":"Exact delegation only","evidence":[],"audit":[{"sequence":1,"at":"2026-10-22T00:00:00Z","kind":"accepted","task_id":null,"attempt":null,"detail":"Local choice"}]})).expect("record")
    }
    fn chain(d: &mut DecisionRecord) {
        for kind in [
            DecisionEventKind::Consumed,
            DecisionEventKind::Implemented,
            DecisionEventKind::ChecksPassed,
            DecisionEventKind::ReviewAccepted,
            DecisionEventKind::TaskVerified,
        ] {
            d.audit.push(DecisionEvent {
                sequence: d.audit.len() + 1,
                at: d.created_at,
                kind,
                task_id: Some("b".into()),
                attempt: Some(2),
                detail: "tree".into(),
            });
        }
    }
    #[test]
    fn verified_requires_ordered_evidence_for_the_same_revision_attempt_and_tree() {
        let mut d = record();
        chain(&mut d);
        assert!(d.verified());
        for kind in [
            DecisionEventKind::Consumed,
            DecisionEventKind::Implemented,
            DecisionEventKind::ChecksPassed,
            DecisionEventKind::ReviewAccepted,
        ] {
            let mut missing = d.clone();
            missing.audit.retain(|e| e.kind != kind);
            assert!(!missing.verified());
        }
        let mut other = d.clone();
        other.audit[3].attempt = Some(1);
        assert!(!other.verified());
        other = d.clone();
        other.audit[3].detail = "other-tree".into();
        assert!(!other.verified());
        other = d.clone();
        other.audit.swap(2, 3);
        other.audit[2].sequence = 3;
        other.audit[3].sequence = 4;
        assert!(!other.verified());
        d.audit.push(DecisionEvent {
            sequence: 7,
            at: d.created_at,
            kind: DecisionEventKind::Superseded,
            task_id: None,
            attempt: None,
            detail: "D-next".into(),
        });
        assert!(!d.verified());
    }
    #[test]
    fn historical_provenance_is_not_retroactively_certified() {
        let e:DecisionProvenance=serde_json::from_value(json!({"file":"b.txt","content_sha256":"hash","excerpt":"candidate","supports":"stable","base_commit":"base","workspace_id":Uuid::nil()})).expect("legacy");
        assert_eq!(e.source, DecisionEvidenceSource::LegacyUnverified);
        assert_eq!(e.blob_oid, None);
    }
    #[test]
    fn independent_review_requires_every_consumed_revision_once() {
        use crate::engineering::ReviewResult;
        let review:ReviewResult=serde_json::from_value(json!({"accepted":true,"criteria":[],"findings":[],"blocker":"","decision_reviews":[{"decision_id":"D-active","implemented":false,"evidence":"The diff implements the historical choice instead."}]})).expect("review");
        assert!(review.validate_decisions(&["D-active".into()]).is_ok());
        assert!(!review.accepts());
        assert!(review.validate_decisions(&["D-historical".into()]).is_err());
        let mut missing = review.clone();
        missing.decision_reviews.clear();
        assert!(missing.validate_decisions(&["D-active".into()]).is_err());
        let mut duplicate = review.clone();
        duplicate
            .decision_reviews
            .push(review.decision_reviews[0].clone());
        assert!(duplicate.validate_decisions(&["D-active".into()]).is_err());
    }
}
