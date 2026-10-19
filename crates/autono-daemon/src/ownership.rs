use std::fs::{File, OpenOptions};

use anyhow::{Context, Result};
use autono_core::config::Paths;
use autono_protocol::DaemonMetadata;
use fs2::FileExt;

/// The file is never removed: unlinking a locked inode would permit two owners.
pub struct Ownership {
    _lock: File,
}

impl Ownership {
    pub async fn acquire(paths: &Paths) -> Result<Self> {
        let path = paths.home.join("daemon.lock");
        let lock = tokio::task::spawn_blocking(move || -> Result<File> {
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc_no_follow());
            }
            let file = options.open(path).context("open daemon lock")?;
            file.try_lock_exclusive()
                .context("another daemon already owns this Autono home")?;
            Ok(file)
        })
        .await??;
        Ok(Self { _lock: lock })
    }

    pub async fn publish(&self, paths: &Paths, metadata: &DaemonMetadata) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let temporary = paths.home.join("daemon.json.tmp");
        // Safe to remove a stale temporary file only while holding the ownership lock.
        match tokio::fs::remove_file(&temporary).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("remove stale metadata temporary file"),
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).await?;
        file.write_all(&serde_json::to_vec(metadata)?).await?;
        file.sync_all().await?;
        tokio::fs::rename(&temporary, paths.metadata())
            .await
            .context("publish daemon metadata")?;
        Ok(())
    }

    pub async fn unpublish(&self, paths: &Paths) -> Result<()> {
        tokio::fs::remove_file(paths.metadata())
            .await
            .context("remove daemon metadata")
    }
}

#[cfg(unix)]
fn libc_no_follow() -> i32 {
    // fs2 is portable; O_NOFOLLOW prevents accidentally locking a symlink target.
    // Values are supplied through std's platform extension by the libc dependency.
    libc::O_NOFOLLOW
}
