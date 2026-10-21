//! Daemon-owned engineering orchestration above the process supervisor.
pub mod agent;
mod artifacts;
mod delegation;
mod engine;
mod git;
mod journal;
mod preflight;
pub mod process;
mod review;
mod scoped_files;
mod service;
mod store;
pub mod workspace;
pub use autono_protocol::engineering::*;
pub use service::EngineeringService;
pub use store::{EngineeringConflict, EngineeringNotFound};
