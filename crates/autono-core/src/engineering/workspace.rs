//! Workspaces are reserved durably before Git operations; never remove or reset them.
use super::{Workspace, process::ProcessRunner};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    path::{Component, Path},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub fn reference(home: &Path, run: Uuid, label: &str, base: &str) -> Workspace {
    let id = Uuid::new_v4();
    Workspace {
        id,
        path: home.join("engineering").join(run.to_string()).join(label),
        branch: format!("autono/{run}/{label}"),
        base_commit: base.into(),
        head_commit: None,
        git: None,
    }
}
pub struct WorkspaceLease {
    _file: File,
}
impl WorkspaceLease {
    /// The lock lives outside the checkout so agents cannot accidentally stage it.
    pub fn acquire(workspace: &Workspace) -> Result<Self> {
        let parent = workspace
            .path
            .parent()
            .context("workspace parent missing")?;
        std::fs::create_dir_all(parent)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join(format!("{}.lock", workspace.id)))?;
        file.try_lock_exclusive()
            .context("workspace already has an active writer")?;
        Ok(Self { _file: file })
    }
}
pub async fn repository(
    runner: &ProcessRunner,
    path: &Path,
    base: &str,
    stop: &CancellationToken,
) -> Result<(std::path::PathBuf, String)> {
    ensure!(
        !base.is_empty() && !base.starts_with('-') && base.len() <= 200 && !base.contains('\0'),
        "invalid Git base reference"
    );
    let path = tokio::fs::canonicalize(path).await?;
    let root = runner
        .git(&path, &["rev-parse", "--show-toplevel"], stop)
        .await?;
    let root = tokio::fs::canonicalize(root.trim()).await?;
    ensure!(root == path, "repository must be its worktree root");
    let oid = runner
        .git(
            &root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{base}^{{commit}}"),
            ],
            stop,
        )
        .await?;
    let oid = oid.trim().to_string();
    ensure!(
        oid.len() >= 40 && oid.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid base commit"
    );
    Ok((root, oid))
}
pub async fn create(
    runner: &ProcessRunner,
    repo: &Path,
    workspace: &mut Workspace,
    stop: &CancellationToken,
) -> Result<()> {
    ensure!(
        !tokio::fs::try_exists(&workspace.path).await?,
        "reserved workspace path already exists; retained for manual inspection"
    );
    let path = workspace.path.to_str().context("non-UTF8 workspace path")?;
    runner
        .git(
            repo,
            &[
                "worktree",
                "add",
                "-b",
                &workspace.branch,
                "--",
                path,
                &workspace.base_commit,
            ],
            stop,
        )
        .await?;
    workspace.git = Some(runner.git_ownership(&workspace.path)?);
    Ok(())
}
pub async fn head(runner: &ProcessRunner, path: &Path, stop: &CancellationToken) -> Result<String> {
    Ok(runner
        .git(path, &["rev-parse", "HEAD"], stop)
        .await?
        .trim()
        .into())
}
pub async fn assert_owned(
    runner: &ProcessRunner,
    workspace: &Workspace,
    expected: &str,
    stop: &CancellationToken,
) -> Result<()> {
    runner.bind_git(
        workspace
            .git
            .as_ref()
            .context("workspace lacks pinned Git ownership; submit a new plan")?,
    )?;
    ensure!(
        head(runner, &workspace.path, stop).await? == expected,
        "workspace HEAD changed outside harness control"
    );
    ensure!(
        runner
            .git(&workspace.path, &["symbolic-ref", "--short", "HEAD"], stop)
            .await?
            .trim()
            == workspace.branch,
        "workspace branch changed outside harness control"
    );
    Ok(())
}
pub async fn merge(
    runner: &ProcessRunner,
    workspace: &Workspace,
    commit: &str,
    stop: &CancellationToken,
) -> Result<()> {
    runner
        .git(
            &workspace.path,
            &["merge", "--no-ff", "--no-edit", "--no-gpg-sign", commit],
            stop,
        )
        .await?;
    Ok(())
}
pub async fn stage(runner: &ProcessRunner, path: &Path, stop: &CancellationToken) -> Result<()> {
    runner.git(path, &["add", "--all", "--", "."], stop).await?;
    Ok(())
}
pub async fn commit(
    runner: &ProcessRunner,
    path: &Path,
    message: &str,
    stop: &CancellationToken,
) -> Result<String> {
    runner
        .git(
            path,
            &["commit", "--allow-empty", "--no-gpg-sign", "-m", message],
            stop,
        )
        .await?;
    head(runner, path, stop).await
}
/// The coder may edit source, but not HEAD/index control state (including index flags).
pub fn git_state(runner: &ProcessRunner, path: &Path) -> Result<String> {
    let owned = runner.git_ownership(path)?;
    let mut digest = Sha256::new();
    for name in ["HEAD", "index"] {
        digest.update(name.as_bytes());
        match std::fs::read(owned.git_dir.join(name)) {
            Ok(bytes) => {
                digest.update([1]);
                digest.update(bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => digest.update([0]),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}
/// Mutation audit covers HEAD, index, tracked and nonignored untracked files, modes and symlinks.
/// This detects changes; it is not an OS sandbox and cannot observe transient/reverted writes.
pub async fn snapshot(
    runner: &ProcessRunner,
    path: &Path,
    stop: &CancellationToken,
) -> Result<String> {
    let root_metadata = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "workspace root was replaced by a symlink or non-directory"
    );
    let root = tokio::fs::canonicalize(path).await?;
    let mut digest = Sha256::new();
    let owned = runner.git_ownership(path)?;
    digest.update(serde_json::to_vec(&owned)?);
    digest.update(git_state(runner, path)?);
    for args in [
        &["rev-parse", "HEAD"][..],
        &["symbolic-ref", "HEAD"][..],
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--cached",
            "--binary",
            "HEAD",
        ][..],
    ] {
        digest.update(runner.git(path, args, stop).await?.as_bytes());
    }
    let files = runner
        .git(
            path,
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
            stop,
        )
        .await?;
    let files: BTreeSet<_> = files.split('\0').filter(|s| !s.is_empty()).collect();
    ensure!(files.len() <= 4096, "workspace audit exceeds 4096 files");
    let mut bytes = 0usize;
    for file in files {
        let relative = Path::new(file);
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "unsafe workspace audit path"
        );
        digest.update((file.len() as u64).to_le_bytes());
        digest.update(file.as_bytes());
        let file = path.join(relative);
        if let Some(parent) = file.parent() {
            match tokio::fs::canonicalize(parent).await {
                Ok(parent) => ensure!(
                    parent.starts_with(&root),
                    "workspace audit path escapes checkout through a symlink"
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    digest.update(b"missing-parent");
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let metadata = match tokio::fs::symlink_metadata(&file).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                digest.update(b"missing");
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            digest.update(metadata.permissions().mode().to_le_bytes());
        }
        if metadata.file_type().is_symlink() {
            digest.update(b"symlink");
            digest.update(
                tokio::fs::read_link(&file)
                    .await?
                    .as_os_str()
                    .as_encoded_bytes(),
            );
        } else {
            ensure!(
                metadata.is_file(),
                "submodules/special files are not supported by workspace auditing"
            );
            bytes = bytes.saturating_add(usize::try_from(metadata.len())?);
            ensure!(bytes <= 64 * 1024 * 1024, "workspace audit exceeds 64 MiB");
            digest.update(metadata.len().to_le_bytes());
            digest.update(tokio::fs::read(&file).await?);
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}
