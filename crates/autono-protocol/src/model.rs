use std::{collections::BTreeMap, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Pending,
                Self::Running | Self::Failed | Self::Cancelled | Self::Interrupted
            ) | (
                Self::Running,
                Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
            )
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::Interrupted => "INTERRUPTED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub timeout_seconds: Option<u64>,
    #[serde(default = "default_keep_awake")]
    pub keep_awake: bool,
}

fn default_keep_awake() -> bool {
    true
}

impl RunSpec {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.command.is_empty() && !self.command[0].is_empty(),
            "command cannot be empty"
        );
        anyhow::ensure!(self.cwd.is_absolute(), "working directory must be absolute");
        anyhow::ensure!(self.timeout_seconds != Some(0), "timeout must be positive");
        anyhow::ensure!(
            self.timeout_seconds.is_none_or(|n| n <= 31_536_000),
            "timeout exceeds one year"
        );
        anyhow::ensure!(
            self.command.iter().all(|s| !s.contains('\0')),
            "command contains a NUL byte"
        );
        anyhow::ensure!(
            self.env
                .iter()
                .all(|(k, v)| !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0')),
            "invalid environment variable"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    #[serde(default)]
    pub output_complete: Option<bool>,
    #[serde(default)]
    pub submission_key: Option<Uuid>,
    #[serde(default)]
    pub cleanup_warnings: Vec<String>,
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: RunStatus,
    pub spec: RunSpec,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Globally increasing durable cursor; gaps between one run's events are valid.
    pub sequence: i64,
    pub run_id: Uuid,
    pub timestamp: DateTime<Utc>,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    Created,
    StateChanged {
        from: RunStatus,
        to: RunStatus,
        detail: Option<String>,
    },
    /// Raw bytes encoded as base64; preserves binary output and split UTF-8 characters.
    Output {
        stream: OutputStream,
        data: String,
    },
    Warning {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_preserve_terminal_states_and_require_start_before_success() {
        let states = [
            RunStatus::Pending,
            RunStatus::Running,
            RunStatus::Succeeded,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::Interrupted,
        ];
        for state in states {
            assert!(!state.can_transition_to(state));
            if state.is_terminal() {
                assert!(states.iter().all(|next| !state.can_transition_to(*next)));
            }
        }
        assert!(!RunStatus::Pending.can_transition_to(RunStatus::Succeeded));
        assert!(RunStatus::Pending.can_transition_to(RunStatus::Running));
        assert!(RunStatus::Running.can_transition_to(RunStatus::Interrupted));
    }
}
