//! Workspaces are reserved durably before Git operations; never remove or reset them.
use super::{Workspace, process::ProcessRunner};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    validate_index(runner, &workspace.path, stop).await?;
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
    validate_index(runner, path, stop).await?;
    runner.git(path, &["add", "--all", "--", "."], stop).await?;
    Ok(())
}
/// Reject preexisting suppression flags, not merely mutations to index bytes.
/// -v exposes assume-unchanged; -f exposes fsmonitor-valid. Skip-worktree,
/// unmerged and sparse entries are also rejected instead of silently normalized.
pub async fn validate_index(
    runner: &ProcessRunner,
    path: &Path,
    stop: &CancellationToken,
) -> Result<()> {
    let owned = runner.git_ownership(path)?;
    ensure!(
        owned.git_dir.join("index").is_file(),
        "missing workspace Git index"
    );
    for flag in ["-v", "-f"] {
        let entries = runner.git(path, &["ls-files", flag, "-z"], stop).await?;
        ensure!(
            entries
                .split('\0')
                .filter(|s| !s.is_empty())
                .all(|s| s.starts_with("H ")),
            "unsafe Git index flags/state; inspect the retained workspace"
        );
    }
    Ok(())
}

/// Establish a safe baseline before any coder is admitted.
pub async fn coder_state(
    runner: &ProcessRunner,
    path: &Path,
    stop: &CancellationToken,
) -> Result<String> {
    validate_index(runner, path, stop).await?;
    git_state(runner, path)
}

/// Bind actual filesystem bytes and executable bits to an immutable staged tree.
/// Deliberately bypass stat-cache shortcuts and all clean/EOL filters. Unsupported
/// transformations, submodules, missing entries and nonignored unstaged files fail
/// closed. This may populate Git's cache-tree extension, but never stages files.
pub async fn candidate_tree(
    runner: &ProcessRunner,
    path: &Path,
    stop: &CancellationToken,
) -> Result<String> {
    validate_index(runner, path, stop).await?;
    let tree = runner
        .git(path, &["write-tree"], stop)
        .await?
        .trim()
        .to_owned();
    // write-tree may populate Git's cache-tree extension. Pin control state after
    // that harness-owned update, and reject mutations during the actual audit.
    let control = git_state(runner, path)?;
    let entries = runner
        .git(path, &["ls-files", "--stage", "-z"], stop)
        .await?;
    // Compare complete manifests, including stage/mode/OID. write-tree and the
    // index read are separate operations; raw control hashes alone cannot bind
    // them. This also rejects intent-to-add entries omitted by write-tree.
    let immutable = runner
        .git(path, &["ls-tree", "-r", "--full-tree", "-z", &tree], stop)
        .await?;
    let mut expected = BTreeMap::new();
    for entry in immutable.split('\0').filter(|s| !s.is_empty()) {
        let (header, name) = entry.split_once('\t').context("invalid tree entry")?;
        let fields: Vec<_> = header.split(' ').collect();
        ensure!(
            fields.len() == 3 && fields[1] == "blob",
            "unsupported candidate tree entry"
        );
        expected.insert(name, (fields[0], fields[2]));
        ensure!(expected.len() <= 4096, "candidate audit exceeds 4096 files");
    }
    let root = tokio::fs::canonicalize(path).await?;
    let mut regular = Vec::new();
    let mut total = 0u64;
    for (index, entry) in entries.split('\0').filter(|s| !s.is_empty()).enumerate() {
        ensure!(index < 4096, "candidate audit exceeds 4096 files");
        let (header, name) = entry.split_once('\t').context("invalid Git index entry")?;
        let fields: Vec<_> = header.split(' ').collect();
        ensure!(fields.len() == 3 && fields[2] == "0", "unmerged Git index");
        ensure!(
            expected.remove(name) == Some((fields[0], fields[1])),
            "index inventory/content/mode differs from immutable candidate tree"
        );
        let relative = Path::new(name);
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "unsafe Git index path"
        );
        let file = path.join(relative);
        ensure!(
            tokio::fs::canonicalize(file.parent().context("missing parent")?)
                .await?
                .starts_with(&root),
            "Git index path escapes checkout"
        );
        let metadata = tokio::fs::symlink_metadata(&file)
            .await
            .context("staged file missing from candidate")?;
        total = total.saturating_add(metadata.len());
        ensure!(total <= 64 * 1024 * 1024, "candidate audit exceeds 64 MiB");
        if metadata.file_type().is_symlink() {
            ensure!(
                fields[0] == "120000",
                "staged symlink mode differs from candidate"
            );
            let blob = runner
                .git(path, &["cat-file", "blob", fields[1]], stop)
                .await?;
            ensure!(
                blob.as_bytes()
                    == tokio::fs::read_link(&file)
                        .await?
                        .as_os_str()
                        .as_encoded_bytes(),
                "staged symlink differs from candidate"
            );
        } else {
            ensure!(metadata.is_file(), "unsupported candidate file type");
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                let bits = metadata.permissions().mode();
                ensure!(
                    (bits & 0o111 != 0) == (bits & 0o100 != 0),
                    "candidate executable bits cannot be represented by Git"
                );
                if bits & 0o100 != 0 {
                    "100755"
                } else {
                    "100644"
                }
            };
            #[cfg(not(unix))]
            let mode = {
                anyhow::bail!("candidate executable-mode auditing requires Unix");
            };
            ensure!(
                fields[0] == mode,
                "staged executable mode differs from candidate: {name}"
            );
            regular.push((name, fields[1]));
        }
    }
    ensure!(expected.is_empty(), "index omits candidate tree files");
    // One process per bounded batch, never one process per source file. Hash the
    // bytes on disk even when Git's stat cache considers the file unchanged.
    for batch in regular.chunks(64) {
        let mut args = vec!["hash-object", "--no-filters", "--"];
        args.extend(batch.iter().map(|(name, _)| *name));
        let hashes = runner.git(path, &args, stop).await?;
        ensure!(
            hashes.lines().eq(batch.iter().map(|(_, oid)| *oid)),
            "staged content differs from candidate"
        );
    }
    ensure!(
        runner
            .git(
                path,
                &["ls-files", "--others", "--exclude-standard", "-z"],
                stop
            )
            .await?
            .is_empty(),
        "candidate contains unstaged files"
    );
    validate_index(runner, path, stop).await?;
    ensure!(
        control == git_state(runner, path)?,
        "Git control state changed during candidate audit"
    );
    Ok(tree)
}

/// Preserve exactly the reviewed immutable tree, rather than letting `commit`
/// reread a mutable index. Publish with compare-and-swap on the owned branch.
pub async fn commit(
    runner: &ProcessRunner,
    workspace: &Workspace,
    tree: &str,
    snapshot: &str,
    message: &str,
    stop: &CancellationToken,
) -> Result<String> {
    let path = &workspace.path;
    assert_owned(runner, workspace, &workspace.base_commit, stop).await?;
    ensure!(
        candidate_tree(runner, path, stop).await? == tree,
        "candidate tree changed after review"
    );
    ensure!(
        self::snapshot(runner, path, stop).await? == snapshot,
        "candidate changed after verification/review"
    );
    let commit = runner
        .git(
            path,
            &[
                "commit-tree",
                tree,
                "-p",
                &workspace.base_commit,
                "-m",
                message,
            ],
            stop,
        )
        .await?
        .trim()
        .to_owned();
    ensure!(
        self::snapshot(runner, path, stop).await? == snapshot,
        "candidate changed during commit creation"
    );
    runner
        .git(
            path,
            &[
                "update-ref",
                &format!("refs/heads/{}", workspace.branch),
                &commit,
                &workspace.base_commit,
            ],
            stop,
        )
        .await?;
    assert_owned(runner, workspace, &commit, stop).await?;
    ensure!(
        runner
            .git(path, &["rev-parse", &format!("{commit}^{{tree}}")], stop)
            .await?
            .trim()
            == tree,
        "preserved commit tree differs from reviewed tree"
    );
    ensure!(
        candidate_tree(runner, path, stop).await? == tree,
        "candidate changed during artifact preservation"
    );
    Ok(commit)
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
    validate_index(runner, path, stop).await?;
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
