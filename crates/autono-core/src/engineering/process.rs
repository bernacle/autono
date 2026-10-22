//! All workflow commands reuse Runtime v0 ownership, closed stdin and process-group cleanup.
use super::git::Ownership;
use super::{GitOwnership, VerificationOutcome, agent::AgentFailure};
use crate::{EventKind, Run, RunSpec, RunStatus, Supervisor, model::OutputStream};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const CAPTURE_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Clone)]
pub struct ProcessRunner {
    pub supervisor: Supervisor,
    git: Arc<Mutex<BTreeMap<PathBuf, GitOwnership>>>,
    deadline: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(Debug)]
pub struct CommandOutput {
    pub run: Run,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}
impl CommandOutput {
    pub fn outcome(&self) -> VerificationOutcome {
        if matches!(
            self.run.status,
            RunStatus::Cancelled | RunStatus::Interrupted
        ) {
            VerificationOutcome::Cancelled
        } else if self.run.detail.as_deref() == Some("timeout exceeded") {
            VerificationOutcome::Timeout
        } else if self.succeeded() {
            VerificationOutcome::Passed
        } else if self.run.status == RunStatus::Failed
            && self.run.exit_code.is_some()
            && self.run.output_complete == Some(true)
        {
            VerificationOutcome::TestFailure
        } else {
            VerificationOutcome::InfrastructureFailure
        }
    }

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
    pub fn new(supervisor: Supervisor) -> Self {
        Self {
            supervisor,
            git: Arc::new(Mutex::new(BTreeMap::new())),
            deadline: None,
        }
    }
    pub fn with_deadline(&self, deadline: Option<chrono::DateTime<chrono::Utc>>) -> Self {
        let mut runner = self.clone();
        // A nested scope may narrow an existing deadline, never extend it.
        runner.deadline = match (self.deadline, deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        runner
    }
    pub(crate) fn check_authority(&self, stop: &CancellationToken) -> Result<()> {
        if self.deadline.is_some_and(|d| chrono::Utc::now() >= d) {
            return Err(super::delegation::denied(
                "total runtime budget exhausted; active processes cancelled, completed evidence retained",
            ));
        }
        anyhow::ensure!(
            !stop.is_cancelled(),
            "workflow cancelled before process admission"
        );
        Ok(())
    }
    pub fn bind_git(&self, owned: &GitOwnership) -> Result<()> {
        owned.verify()?;
        let mut map = self.git.lock().expect("Git bindings");
        if let Some(previous) = map.get(&owned.worktree) {
            anyhow::ensure!(previous == owned, "Git ownership binding changed");
        } else {
            map.insert(owned.worktree.clone(), owned.clone());
        }
        Ok(())
    }
    pub fn git_ownership(&self, cwd: &Path) -> Result<GitOwnership> {
        // Consult the trusted spelling before canonicalization can follow a replaced path.
        if let Some(owned) = self.git.lock().expect("Git bindings").get(cwd).cloned() {
            owned.verify()?;
            return Ok(owned);
        }
        let path = std::fs::canonicalize(cwd)?;
        let previous = self.git.lock().expect("Git bindings").get(&path).cloned();
        let owned = match previous {
            Some(owned) => owned,
            None => GitOwnership::capture(&path)?,
        };
        self.bind_git(&owned)?;
        self.git
            .lock()
            .expect("Git bindings")
            .insert(cwd.to_owned(), owned.clone());
        Ok(owned)
    }

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
        self.check_authority(stop)?;
        let spec = RunSpec {
            command,
            cwd: cwd.to_owned(),
            env: BTreeMap::from([
                ("GIT_TERMINAL_PROMPT".into(), "0".into()),
                ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
                ("GIT_EDITOR".into(), "true".into()),
                ("GIT_MERGE_AUTOEDIT".into(), "no".into()),
            ]),
            timeout_seconds: Some(seconds),
            keep_awake: true,
        };
        self.start_spec(spec, key, stop).await
    }
    async fn start_spec(&self, spec: RunSpec, key: Uuid, stop: &CancellationToken) -> Result<Run> {
        self.check_authority(stop)?;
        let authority = crate::supervisor::ExecutionAuthority {
            cancel: stop.clone(),
            deadline: self.deadline,
        };
        let mut delay = Duration::from_millis(50);
        let mut retries = 0;
        let run = loop {
            self.check_authority(stop)?;
            match self
                .supervisor
                .submit_authorized(
                    autono_protocol::Submission {
                        submission_key: key,
                        spec: spec.clone(),
                    },
                    authority.clone(),
                )
                .await
            {
                Ok(run) => break run,
                Err(e) if e.is::<crate::supervisor::Overloaded>() && retries < 6 => {
                    self.check_authority(stop)?;
                    retries += 1;
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => { self.check_authority(stop)?; anyhow::bail!("workflow cancelled during admission backoff"); },
                        _ = authority.deadline_reached() => return Err(super::delegation::denied("total runtime budget exhausted during process admission backoff")),
                        _ = tokio::time::sleep(delay) => {},
                    }
                    delay *= 2;
                }
                Err(e) if e.is::<crate::supervisor::Overloaded>() => {
                    self.check_authority(stop)?;
                    return Err(AgentFailure {
                        kind: "process_admission".into(),
                        detail: format!(
                            "Process capacity unavailable after {retries} retries: {e}"
                        ),
                        blocked: true,
                    }
                    .into());
                }
                Err(e) => return Err(e),
            }
        };
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
            // Ledger checkpoints and polling clients can briefly occupy the bounded
            // reader pool. Retry reads only; never resubmit a process or effect.
            let run = crate::persistence::retry(|| store.get(id)).await?;
            let events = crate::persistence::retry(|| store.events(id, cursor, 500)).await?;
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
        // Bootstrap is only permitted before any agents run, when creating a new repository.
        let initializing = args.first() == Some(&"init") && !cwd.join(".git").exists();
        let owned = if initializing {
            None
        } else {
            Some(self.git_ownership(cwd)?)
        };
        let mut command = vec![
            "git".into(),
            "--no-pager".into(),
            "--no-optional-locks".into(),
        ];
        if let Some(owned) = &owned {
            command.extend([
                format!("--git-dir={}", owned.git_dir.display()),
                format!("--work-tree={}", owned.worktree.display()),
            ]);
        }
        for setting in [
            "core.hooksPath=/dev/null",
            // Command-line config also reaches worktree-add's child checkout.
            // These must apply BEFORE Git creates an index, not just at staging.
            "core.ignoreStat=false",
            "core.fileMode=true",
            "core.fsmonitor=false",
            "core.untrackedCache=false",
            "core.attributesFile=/dev/null",
            "core.sshCommand=false",
            "credential.helper=",
            "core.askPass=",
            "core.bare=false",
            "commit.gpgSign=false",
            "tag.gpgSign=false",
            "protocol.ext.allow=never",
            "protocol.file.allow=never",
            "merge.autoStash=false",
            "merge.verifySignatures=false",
            "log.showSignature=false",
            "gpg.program=false",
            "gpg.ssh.program=false",
            "user.name=Autono",
            "user.email=autono@localhost",
        ] {
            command.extend(["-c".into(), setting.into()]);
        }
        command.extend(args.iter().map(|s| s.to_string()));
        // Neutralize inherited Git configuration and execution environment. Explicit
        // directories above and audited repository config define harness ownership.
        let env = BTreeMap::from([
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_SYSTEM".into(), "/dev/null".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_COUNT".into(), "0".into()),
            ("GIT_CONFIG_PARAMETERS".into(), String::new()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
            ("GIT_ATTR_NOSYSTEM".into(), "1".into()),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES".into(), String::new()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_EDITOR".into(), "true".into()),
            ("GIT_MERGE_AUTOEDIT".into(), "no".into()),
        ]);
        // Never leak a parent's GIT_INDEX_FILE into worktree-add's child checkout.
        // Explicit directories and audited commondir own discovery instead.
        let mut isolated = vec!["env".to_string()];
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_NAMESPACE",
            "GIT_SHALLOW_FILE",
            "GIT_CONFIG",
            "GIT_EXTERNAL_DIFF",
            "GIT_DIFF_OPTS",
            "GIT_TEMPLATE_DIR",
            "GIT_EXEC_PATH",
        ] {
            isolated.extend(["-u".into(), key.into()]);
        }
        isolated.extend(command);
        let command = isolated;
        let spec = RunSpec {
            command,
            cwd: cwd.to_owned(),
            env,
            timeout_seconds: Some(60),
            keep_awake: true,
        };
        let run = self.start_spec(spec, Uuid::new_v4(), stop).await?;
        let output = self.wait(run.id, stop).await?;
        anyhow::ensure!(
            output.succeeded(),
            "Git operation failed: {}",
            output.summary()
        );
        if initializing {
            self.git_ownership(cwd)?;
        }
        output.stdout_text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transient_evidence_reader_contention_does_not_replay_or_cancel_a_process() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let store = crate::Store::open(&dir.path().join("db")).await?;
        let config = crate::config::Config::default();
        let supervisor = Supervisor::configured(store.clone(), &config.runner);
        let runner = ProcessRunner::new(supervisor.clone());
        let stop = CancellationToken::new();
        let run = runner
            .start(
                vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    "sleep 0.5; printf evidence".into(),
                ],
                dir.path(),
                5,
                Uuid::new_v4(),
                &stop,
            )
            .await?;
        let mut readers = Vec::new();
        for _ in 0..4 {
            readers.push(store.readers.acquire().await?);
        }
        let waiting = runner.wait(run.id, &stop);
        let release = async {
            tokio::time::sleep(Duration::from_millis(400)).await;
            drop(readers);
        };
        let (output, ()) = tokio::join!(waiting, release);
        let output = output?;
        assert!(output.succeeded());
        assert_eq!(output.stdout_text()?, "evidence");
        assert!(!stop.is_cancelled());
        assert_eq!(
            store.list(10, 0).await?.len(),
            1,
            "read retries must not resubmit the process"
        );
        supervisor.shutdown().await?;
        Ok(())
    }
}
