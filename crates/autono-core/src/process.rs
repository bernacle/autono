//! OS ownership is retained until group signalling is finished. Never signal after reap.
use std::process::ExitStatus;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};

pub(crate) struct Process {
    pub child: Child,
    pub pid: u32,
    owned: bool,
}

impl Process {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        #[cfg(unix)]
        command.process_group(0).kill_on_drop(false);
        #[cfg(not(unix))]
        command.kill_on_drop(true);
        let child = command.spawn().context("spawn command")?;
        let pid = child.id().context("spawned process has no PID")?;
        Ok(Self {
            child,
            pid,
            owned: true,
        })
    }

    #[cfg(unix)]
    fn observe(&mut self) -> Result<Option<ExitStatus>> {
        use std::os::unix::process::ExitStatusExt;
        // SAFETY: siginfo_t is a C POD; waitid initializes it. WNOWAIT retains the
        // child/zombie and pins its PID/PGID until our explicit reap below.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                self.owned = false;
            }
            return Err(error).context("observe owned child without reaping");
        }
        // SAFETY: waitid populated the SIGCHLD fields (or left si_pid zero).
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        let code = unsafe { info.si_status() };
        let raw = if info.si_code == libc::CLD_EXITED {
            code << 8
        } else {
            code | if info.si_code == libc::CLD_DUMPED {
                0x80
            } else {
                0
            }
        };
        Ok(Some(ExitStatus::from_raw(raw)))
    }

    #[cfg(unix)]
    pub async fn exited(&mut self) -> Result<ExitStatus> {
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::child())?;
        loop {
            if let Some(status) = self.observe()? {
                return Ok(status);
            }
            signal.recv().await.context("SIGCHLD stream closed")?;
        }
    }

    #[cfg(not(unix))]
    pub async fn exited(&mut self) -> Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.owned = false;
        Ok(status)
    }

    pub fn terminate(&mut self) -> Result<()> {
        if !self.owned {
            return Ok(());
        }
        #[cfg(unix)]
        {
            // Verify the child is still waitable before using its group. ECHILD
            // revokes ownership; no persisted or already-reaped identifier is used.
            self.observe()?;
            let group = i32::try_from(self.pid)?;
            // SAFETY: the live or unreaped child pins this dedicated group ID.
            if unsafe { libc::kill(-group, libc::SIGKILL) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error).context("terminate pinned process group");
                }
            }
        }
        #[cfg(not(unix))]
        self.child.start_kill()?;
        Ok(())
    }

    pub async fn reap(&mut self) -> Result<ExitStatus> {
        // Revoke the group capability BEFORE the only consuming wait operation.
        self.owned = false;
        self.child.wait().await.context("reap direct child")
    }

    pub fn kill_direct(&mut self) -> Result<()> {
        if self.owned {
            self.child.start_kill().context("terminate direct child")?;
        }
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.owned
            && let Err(error) = self.terminate()
        {
            tracing::error!(%error, "owned process cleanup failed");
        }
    }
}
