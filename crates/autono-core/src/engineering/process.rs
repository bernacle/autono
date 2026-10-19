//! All workflow commands reuse Runtime v0 ownership, closed stdin and process-group cleanup.
use crate::{EventKind, Run, RunSpec, RunStatus, Supervisor, model::OutputStream};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const CAPTURE_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Clone)]
pub struct ProcessRunner {
    pub supervisor: Supervisor,
}
#[derive(Debug)]
pub struct CommandOutput {
    pub run: Run,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}
impl CommandOutput {
    pub fn succeeded(&self) -> bool {
        self.run.status == RunStatus::Succeeded
            && self.run.exit_code == Some(0)
            && self.run.output_complete == Some(true)
    }
    pub fn stdout_text(&self) -> Result<String> {
        anyhow::ensure!(
            !self.truncated,
            "command output exceeds capture limit; inspect process logs {}",
            self.run.id
        );
        String::from_utf8(self.stdout.clone()).context("command output is not UTF-8")
    }
    pub fn summary(&self) -> String {
        let text = String::from_utf8_lossy(&self.stderr);
        let out = String::from_utf8_lossy(&self.stdout);
        let tail = if text.is_empty() { &out } else { &text };
        let tail: String = tail
            .chars()
            .rev()
            .take(3000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!(
            "process={} status={:?} exit={:?} output_complete={:?} warnings={:?}; {}{}",
            self.run.id,
            self.run.status,
            self.run.exit_code,
            self.run.output_complete,
            self.run.cleanup_warnings,
            self.run.detail.as_deref().unwrap_or(""),
            tail
        )
    }
}
impl ProcessRunner {
    pub async fn run(
        &self,
        command: Vec<String>,
        cwd: &Path,
        seconds: u64,
        key: Uuid,
        stop: &CancellationToken,
    ) -> Result<CommandOutput> {
        let run = self.start(command, cwd, seconds, key, stop).await?;
        self.wait(run.id, stop).await
    }
    /// Runtime v0 owns the process after acceptance, even if the caller disconnects.
    pub async fn start(
        &self,
        command: Vec<String>,
        cwd: &Path,
        seconds: u64,
        key: Uuid,
        stop: &CancellationToken,
    ) -> Result<Run> {
        anyhow::ensure!(
            !stop.is_cancelled(),
            "workflow cancelled before process launch"
        );
        let spec = RunSpec {
            command,
            cwd: cwd.to_owned(),
            env: BTreeMap::from([
                ("GIT_TERMINAL_PROMPT".into(), "0".into()),
                ("GIT_EDITOR".into(), "true".into()),
                ("GIT_MERGE_AUTOEDIT".into(), "no".into()),
            ]),
            timeout_seconds: Some(seconds),
            keep_awake: true,
        };
        let run = self
            .supervisor
            .submit_keyed(autono_protocol::Submission {
                submission_key: key,
                spec,
            })
            .await?;
        if let Some(finished) = self.supervisor.control_finished(run.id) {
            let supervisor = self.supervisor.clone();
            let stop = stop.clone();
            let id = run.id;
            // Cancellation cannot depend on a workflow checkpoint acquiring SQLite.
            // This bounded-lifetime watcher ends with process control, before finalization.
            tokio::spawn(async move {
                tokio::select! {
                    biased;
                    _ = finished.cancelled() => {},
                    _ = stop.cancelled() => { let _ = supervisor.cancel(id).await; },
                }
            });
        }
        Ok(run)
    }
    /// On read failure, stop and finish process control before releasing caller ownership.
    pub async fn wait(&self, id: Uuid, stop: &CancellationToken) -> Result<CommandOutput> {
        let control_finished = self.supervisor.control_finished(id);
        let result = self.collect(id, stop).await;
        if result.is_err() {
            stop.cancel();
            let _ = self.supervisor.cancel(id).await;
            if let Some(finished) = control_finished {
                tokio::time::timeout(Duration::from_secs(10), finished.cancelled())
                    .await
                    .context("process control did not finish after workflow read failure")?;
            }
        }
        result
    }
    async fn collect(&self, id: Uuid, stop: &CancellationToken) -> Result<CommandOutput> {
        let store = self.supervisor.store();
        let mut changed = store.subscribe();
        let mut cursor = 0;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut truncated = false;
        let mut cancelled = false;
        loop {
            let run = store.get(id).await?;
            let events = store.events(id, cursor, 500).await?;
            let empty = events.is_empty();
            for event in events {
                cursor = event.sequence;
                if let EventKind::Output { stream, data } = event.kind {
                    let bytes = STANDARD.decode(data)?;
                    let target = if matches!(stream, OutputStream::Stdout) {
                        &mut stdout
                    } else {
                        &mut stderr
                    };
                    let available = CAPTURE_LIMIT.saturating_sub(target.len());
                    truncated |= bytes.len() > available;
                    target.extend_from_slice(&bytes[..bytes.len().min(available)]);
                }
            }
            // Read state before events: terminal commit and output precede a terminal read.
            if run.status.is_terminal() && empty {
                return Ok(CommandOutput {
                    run,
                    stdout,
                    stderr,
                    truncated,
                });
            }
            if !empty {
                continue;
            }
            tokio::select! {
                biased;
                _ = stop.cancelled(), if !cancelled => { cancelled = true; let _ = self.supervisor.cancel(id).await; }
                result = changed.changed() => { result.context("runtime notifications closed")?; }
                // Notifications are hints. A bounded fallback handles independent store handles.
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }
    }
    pub async fn git(&self, cwd: &Path, args: &[&str], stop: &CancellationToken) -> Result<String> {
        let command = std::iter::once("git".to_string())
            .chain(
                [
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "user.name=Autono",
                    "-c",
                    "user.email=autono@localhost",
                ]
                .map(String::from),
            )
            .chain(args.iter().map(|s| s.to_string()))
            .collect();
        let output = self.run(command, cwd, 60, Uuid::new_v4(), stop).await?;
        anyhow::ensure!(
            output.succeeded(),
            "Git operation failed: {}",
            output.summary()
        );
        output.stdout_text()
    }
}
