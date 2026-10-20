use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use autono_protocol::engineering::{
    AgentsConfig, ExecutionConfig, PermissionPolicy, PreflightConfig, VerificationConfig,
    WorkflowSettings,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    pub fn discover(override_home: Option<PathBuf>) -> Result<Self> {
        let home = override_home
            .or_else(|| std::env::var_os("AUTONO_HOME").map(PathBuf::from))
            .or_else(|| {
                directories::ProjectDirs::from("", "", "autono")
                    .map(|dirs| dirs.data_local_dir().to_owned())
            })
            .context("cannot locate user data directory; set AUTONO_HOME or --home")?;
        anyhow::ensure!(home.is_absolute(), "Autono home must be an absolute path");
        Ok(Self { home })
    }

    pub fn database(&self) -> PathBuf {
        self.home.join("runtime.sqlite3")
    }
    pub fn metadata(&self) -> PathBuf {
        self.home.join("daemon.json")
    }
    pub fn config(&self) -> PathBuf {
        self.home.join("config.toml")
    }

    pub async fn prepare(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.home)
            .await
            .context("create Autono home")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let metadata = tokio::fs::symlink_metadata(&self.home).await?;
            anyhow::ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Autono home must be a real directory"
            );
            // SAFETY: geteuid takes no arguments and has no safety preconditions.
            anyhow::ensure!(
                metadata.uid() == unsafe { libc::geteuid() },
                "Autono home must be owned by the current user"
            );
            tokio::fs::set_permissions(&self.home, std::fs::Permissions::from_mode(0o700)).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub runner: RunnerConfig,
    pub daemon: DaemonConfig,
    pub agents: AgentsConfig,
    pub execution: ExecutionConfig,
    pub verification: VerificationConfig,
    pub preflight: PreflightConfig,
    pub permissions: PermissionPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agents: AgentsConfig::default(),
            execution: ExecutionConfig::default(),
            verification: VerificationConfig::default(),
            preflight: PreflightConfig::default(),
            permissions: PermissionPolicy::default(),
            runner: RunnerConfig {
                keep_awake: true,
                max_active_runs: 8,
                max_pending_submissions: 32,
            },
            daemon: DaemonConfig {
                port: 0,
                body_timeout_seconds: 5,
                drain_timeout_seconds: 2,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    pub keep_awake: bool,
    pub max_active_runs: usize,
    pub max_pending_submissions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    pub port: u16,
    pub body_timeout_seconds: u64,
    pub drain_timeout_seconds: u64,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Patch {
    runner: RunnerPatch,
    daemon: DaemonPatch,
    agents: AgentsPatch,
    execution: ExecutionPatch,
    verification: VerificationPatch,
    preflight: Option<PreflightConfig>,
    permissions: Option<PermissionPolicy>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AgentsPatch {
    planner: AgentPatch,
    coder: AgentPatch,
    reviewer: AgentPatch,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AgentPatch {
    provider: Option<autono_protocol::engineering::Provider>,
    model: Option<String>,
    timeout_seconds: Option<u64>,
}
impl AgentPatch {
    fn apply(self, agent: &mut autono_protocol::engineering::AgentConfig) {
        if let Some(value) = self.provider {
            agent.provider = value;
        }
        if let Some(value) = self.model {
            agent.model = Some(value);
        }
        if let Some(value) = self.timeout_seconds {
            agent.timeout_seconds = value;
        }
    }
}
impl AgentsPatch {
    fn apply(self, agents: &mut AgentsConfig) {
        self.planner.apply(&mut agents.planner);
        self.coder.apply(&mut agents.coder);
        self.reviewer.apply(&mut agents.reviewer);
    }
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ExecutionPatch {
    max_parallel: Option<usize>,
    max_attempts: Option<usize>,
    max_review_cycles: Option<usize>,
}
impl ExecutionPatch {
    fn apply(self, config: &mut ExecutionConfig) {
        if let Some(v) = self.max_parallel {
            config.max_parallel = v;
        }
        if let Some(v) = self.max_attempts {
            config.max_attempts = v;
        }
        if let Some(v) = self.max_review_cycles {
            config.max_review_cycles = v;
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct VerificationPatch {
    commands: Option<Vec<Vec<String>>>,
    timeout_seconds: Option<u64>,
}
impl VerificationPatch {
    fn apply(self, config: &mut VerificationConfig) {
        if let Some(v) = self.commands {
            config.commands = v;
        }
        if let Some(v) = self.timeout_seconds {
            config.timeout_seconds = v;
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RunnerPatch {
    keep_awake: Option<bool>,
    max_active_runs: Option<usize>,
    max_pending_submissions: Option<usize>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DaemonPatch {
    port: Option<u16>,
    body_timeout_seconds: Option<u64>,
    drain_timeout_seconds: Option<u64>,
}

impl Config {
    pub fn workflow(&self) -> WorkflowSettings {
        WorkflowSettings {
            agents: self.agents.clone(),
            execution: self.execution.clone(),
            verification: self.verification.clone(),
            preflight: self.preflight.clone(),
            permissions: self.permissions.clone(),
        }
    }
    pub async fn load(paths: &Paths) -> Result<Self> {
        let patch = read_patch(&paths.config()).await?;
        let mut config = Self::default();
        patch.agents.apply(&mut config.agents);
        patch.execution.apply(&mut config.execution);
        patch.verification.apply(&mut config.verification);
        if let Some(v) = patch.preflight {
            config.preflight = v;
        }
        if let Some(v) = patch.permissions {
            config.permissions = v;
        }
        if let Some(value) = patch.runner.keep_awake {
            config.runner.keep_awake = value;
        }
        if let Some(value) = patch.daemon.port {
            config.daemon.port = value;
        }
        if let Some(value) = patch.runner.max_active_runs {
            config.runner.max_active_runs = value;
        }
        if let Some(value) = patch.runner.max_pending_submissions {
            config.runner.max_pending_submissions = value;
        }
        if let Some(value) = patch.daemon.body_timeout_seconds {
            config.daemon.body_timeout_seconds = value;
        }
        if let Some(value) = patch.daemon.drain_timeout_seconds {
            config.daemon.drain_timeout_seconds = value;
        }
        anyhow::ensure!(
            (1..=1024).contains(&config.runner.max_active_runs),
            "max_active_runs must be 1..1024"
        );
        anyhow::ensure!(
            (1..=4096).contains(&config.runner.max_pending_submissions),
            "max_pending_submissions must be 1..4096"
        );
        anyhow::ensure!(
            (1..=60).contains(&config.daemon.body_timeout_seconds),
            "body_timeout_seconds must be 1..60"
        );
        anyhow::ensure!(
            (1..=30).contains(&config.daemon.drain_timeout_seconds),
            "drain_timeout_seconds must be 1..30"
        );
        config.workflow().validate()?;
        Ok(config)
    }

    /// Project configuration can opt runs out of inhibition, but cannot change daemon settings.
    pub async fn with_project(mut self, directory: &Path) -> Result<Self> {
        let patch = read_patch(&directory.join("autono.toml")).await?;
        anyhow::ensure!(
            patch.permissions.is_none()
                && patch.daemon.port.is_none()
                && patch.daemon.body_timeout_seconds.is_none()
                && patch.daemon.drain_timeout_seconds.is_none()
                && patch.runner.max_active_runs.is_none()
                && patch.runner.max_pending_submissions.is_none(),
            "project autono.toml cannot configure daemon settings, process admission limits or permission policy"
        );
        if let Some(value) = patch.runner.keep_awake {
            self.runner.keep_awake = value;
        }
        patch.agents.apply(&mut self.agents);
        patch.execution.apply(&mut self.execution);
        patch.verification.apply(&mut self.verification);
        if let Some(v) = patch.preflight {
            self.preflight = v;
        }
        if let Some(v) = patch.permissions {
            self.permissions = v;
        }
        self.workflow().validate()?;
        Ok(self)
    }
}

async fn read_patch(path: &Path) -> Result<Patch> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parse {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Patch::default()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

pub const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
