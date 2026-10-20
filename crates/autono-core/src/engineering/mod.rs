//! Daemon-owned engineering orchestration above the process supervisor.
pub mod agent;
mod artifacts;
mod engine;
mod git;
mod journal;
pub mod process;
mod review;
mod service;
mod store;
pub mod workspace;
pub use autono_protocol::engineering::*;
pub use service::EngineeringService;
pub use store::{EngineeringConflict, EngineeringNotFound};
