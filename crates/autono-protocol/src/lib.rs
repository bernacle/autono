//! Versioned local API contract. Output event payloads are base64-encoded bytes.
pub mod engineering;
mod model;
pub use model::*;
use serde::{Deserialize, Serialize};

pub const API_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonMetadata {
    pub api_version: u32,
    pub pid: u32,
    pub address: std::net::SocketAddr,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStatus {
    pub api_version: u32,
    pub pid: u32,
    pub platform: String,
    pub database: String,
    pub admission: AdmissionStatus,
    pub power: PowerStatus,
    pub persistence: PersistenceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelResponse {
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub submission_key: uuid::Uuid,
    pub spec: RunSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionStatus {
    /// Occupied capacity, including launch reservations and finalizations.
    pub active_runs: usize,
    pub executing_runs: usize,
    pub finalizing_runs: usize,
    pub pending_submissions: usize,
    pub max_active_runs: usize,
    pub max_pending_submissions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunPhase {
    Starting,
    Executing,
    Finalizing,
    RecoveryRequired,
}

impl RunPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Executing => "EXECUTING",
            Self::Finalizing => "FINALIZING",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedResult {
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub detail: Option<String>,
    pub cleanup_warnings: Vec<String>,
    /// None while output completion is still being determined.
    pub output_complete: Option<bool>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOwnership {
    pub run_id: uuid::Uuid,
    pub phase: RunPhase,
    pub observed_result: Option<ObservedResult>,
    pub persistence_error: Option<String>,
    pub failed_attempts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceStatus {
    pub healthy: bool,
    pub owned_runs: Vec<RunOwnership>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunView {
    #[serde(flatten)]
    pub run: Run,
    /// Live ownership is separate from the last committed SQLite state.
    pub runtime: Option<RunOwnership>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerStatus {
    pub enabled: bool,
    pub eligible_runs: usize,
    pub held: bool,
    pub helper_pid: Option<u32>,
    pub last_error: Option<String>,
}

pub mod decisions;
