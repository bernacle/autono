use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::Result;
pub use autono_protocol::PowerStatus;
use tokio::sync::Mutex;

pub trait PowerLease: Send {
    /// Check helper/OS assertion health without blocking.
    fn check(&self) -> Result<()>;
    fn helper_pid(&self) -> Option<u32> {
        None
    }
}
pub type Inhibition = Box<dyn PowerLease>;
pub type Acquisition<'a> = Pin<Box<dyn Future<Output = Result<Inhibition>> + Send + 'a>>;

/// Dropping the returned handle releases inhibition. Backends must not inhibit the display.
pub trait PowerBackend: Send + Sync {
    fn acquire(&self) -> Acquisition<'_>;
    fn can_retry(&self) -> bool {
        true
    }
}

pub struct SystemPower;

impl PowerBackend for SystemPower {
    fn can_retry(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn acquire(&self) -> Acquisition<'_> {
        Box::pin(async {
            #[cfg(target_os = "macos")]
            {
                use anyhow::Context;
                use std::process::Stdio;
                let mut child = tokio::process::Command::new("/usr/bin/caffeinate")
                    .args(["-i", "-w", &std::process::id().to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .context("start caffeinate idle-system-sleep assertion")?;
                let pid = child.id();
                let (errors, health) = tokio::sync::watch::channel(None);
                let task = tokio::spawn(async move {
                    let message = match child.wait().await {
                        Ok(status) => format!("caffeinate exited unexpectedly: {status}"),
                        Err(error) => format!("caffeinate supervision failed: {error}"),
                    };
                    tracing::warn!(%message, "idle sleep inhibition lost");
                    errors.send_replace(Some(message));
                });
                Ok(Box::new(CaffeinateLease { task, health, pid }) as Inhibition)
            }
            #[cfg(not(target_os = "macos"))]
            {
                anyhow::bail!("idle sleep inhibition is unsupported on this platform")
            }
        })
    }
}

#[cfg(target_os = "macos")]
struct CaffeinateLease {
    pid: Option<u32>,
    task: tokio::task::JoinHandle<()>,
    health: tokio::sync::watch::Receiver<Option<String>>,
}

#[cfg(target_os = "macos")]
impl PowerLease for CaffeinateLease {
    fn helper_pid(&self) -> Option<u32> {
        self.pid
    }
    fn check(&self) -> Result<()> {
        if let Some(message) = self.health.borrow().as_ref() {
            anyhow::bail!("{message}");
        }
        anyhow::ensure!(
            !self.task.is_finished(),
            "caffeinate supervision task ended"
        );
        Ok(())
    }
}

#[cfg(target_os = "macos")]
impl Drop for CaffeinateLease {
    fn drop(&mut self) {
        // Aborting drops Child, whose kill_on_drop stops the helper. -w also makes it
        // release its assertion if the daemon is killed before Drop can run.
        self.task.abort();
    }
}

struct State {
    eligible: usize,
    handle: Option<Inhibition>,
    last_error: Option<String>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

/// One assertion shared by all eligible runs. Acquisition errors never fail a run.
pub struct StayAwake {
    enabled: bool,
    backend: Arc<dyn PowerBackend>,
    state: Arc<Mutex<State>>,
    errors: tokio::sync::watch::Sender<Option<String>>,
}

impl StayAwake {
    pub fn new(enabled: bool, backend: Arc<dyn PowerBackend>) -> Self {
        Self {
            enabled,
            backend,
            state: Arc::new(Mutex::new(State {
                eligible: 0,
                handle: None,
                last_error: None,
                worker: None,
            })),
            errors: tokio::sync::watch::channel(None).0,
        }
    }

    pub async fn enter(&self) -> Option<String> {
        let mut state = self.state.lock().await;
        state.eligible += 1;
        let previous = state.last_error.clone();
        check_health(&mut state);
        if state.last_error != previous {
            self.errors.send_replace(state.last_error.clone());
        }
        if self.enabled && state.handle.is_none() {
            match acquire(&self.backend).await {
                Ok(handle) => {
                    state.handle = Some(handle);
                    state.last_error = None;
                }
                Err(error) => {
                    state.last_error = Some(format!("stay-awake unavailable: {error:#}"));
                }
            }
        }
        if self.enabled && self.backend.can_retry() && state.worker.is_none() {
            let state_ref = Arc::downgrade(&self.state);
            let backend = self.backend.clone();
            let errors = self.errors.clone();
            state.worker = Some(tokio::spawn(async move {
                let mut backoff = std::time::Duration::from_millis(100);
                loop {
                    tokio::time::sleep(backoff).await;
                    let Some(state) = state_ref.upgrade() else {
                        break;
                    };
                    let mut state = state.lock().await;
                    if state.eligible == 0 {
                        break;
                    }
                    let had_handle = state.handle.is_some();
                    check_health(&mut state);
                    if had_handle && state.handle.is_none() {
                        errors.send_replace(state.last_error.clone());
                    }
                    if state.handle.is_none() {
                        match acquire(&backend).await {
                            Ok(handle) => {
                                state.handle = Some(handle);
                                errors.send_replace(Some(
                                    "stay-awake restored after helper failure".into(),
                                ));
                                backoff = std::time::Duration::from_millis(100);
                            }
                            Err(error) => {
                                let message = format!("stay-awake reacquisition failed: {error:#}");
                                tracing::warn!(%message);
                                state.last_error = Some(message.clone());
                                errors.send_replace(Some(message));
                                backoff = (backoff * 2).min(std::time::Duration::from_secs(2));
                            }
                        }
                    }
                }
            }));
        }
        state.last_error.clone()
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Option<String>> {
        self.errors.subscribe()
    }

    pub async fn shutdown(&self) {
        let mut state = self.state.lock().await;
        if let Some(worker) = state.worker.take() {
            worker.abort();
        }
        state.handle = None;
    }

    pub async fn leave(&self) {
        let mut state = self.state.lock().await;
        state.eligible = state.eligible.saturating_sub(1);
        if state.eligible == 0 {
            state.handle = None;
            if let Some(worker) = state.worker.take() {
                worker.abort();
            }
        }
    }

    pub async fn status(&self) -> PowerStatus {
        let mut state = self.state.lock().await;
        let previous = state.last_error.clone();
        check_health(&mut state);
        if state.last_error != previous {
            self.errors.send_replace(state.last_error.clone());
        }
        PowerStatus {
            enabled: self.enabled,
            eligible_runs: state.eligible,
            held: state.handle.is_some(),
            helper_pid: state.handle.as_ref().and_then(|lease| lease.helper_pid()),
            last_error: state.last_error.clone(),
        }
    }
}

fn check_health(state: &mut State) {
    if let Some(error) = state
        .handle
        .as_ref()
        .and_then(|handle| handle.check().err())
    {
        state.last_error = Some(format!("stay-awake lost: {error:#}"));
        state.handle = None;
    }
}

pub async fn capability() -> &'static str {
    if cfg!(target_os = "macos") {
        if tokio::fs::metadata("/usr/bin/caffeinate").await.is_ok() {
            "available (caffeinate -i; display may sleep)"
        } else {
            "unavailable (caffeinate missing)"
        }
    } else {
        "unsupported (runs still work)"
    }
}

async fn acquire(backend: &Arc<dyn PowerBackend>) -> Result<Inhibition> {
    tokio::time::timeout(std::time::Duration::from_secs(1), backend.acquire())
        .await
        .map_err(|_| anyhow::anyhow!("power acquisition timed out"))?
}
