//! Git ownership is captured before an agent runs, never rediscovered from its .git file.
use super::{GitOwnership, agent::AgentFailure};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn regular(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            ensure!(
                m.is_file() && !m.file_type().is_symlink(),
                "Git metadata is not a regular file: {}",
                path.display()
            );
            ensure!(m.len() <= 1024 * 1024, "Git metadata exceeds limit");
            Ok(Some(std::fs::read(path)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn hash(path: &Path) -> Result<Option<String>> {
    Ok(regular(path)?.map(|b| format!("{:x}", Sha256::digest(b))))
}
fn resolve(parent: &Path, value: &str) -> Result<PathBuf> {
    Ok(std::fs::canonicalize(parent.join(value.trim()))?)
}
fn safe_config(path: &Path) -> Result<()> {
    let Some(bytes) = regular(path)? else {
        return Ok(());
    };
    let text = std::str::from_utf8(&bytes)?;
    // Reject lazy-fetch configuration before any Git probe. Even config key
    // names can embed credential-bearing URLs in subsections; do not collect
    // them through a logged process. Git key spellings cannot be escaped.
    let lower = text.to_ascii_lowercase();
    ensure!(
        !lower.contains("partialclone") && !lower.contains("promisor"),
        "partial-clone discovery is unsupported; materialize repository objects externally before delegation"
    );
    for line in text.lines() {
        let line = line.trim().to_ascii_lowercase();
        if let Some(header) = line.strip_prefix('[') {
            let section = header.split([' ', '\t', ']', '.']).next().unwrap_or("");
            // Fail closed, including inline keys and deprecated dotted subsections.
            ensure!(
                !matches!(
                    section,
                    "include" | "includeif" | "filter" | "merge" | "diff"
                ),
                "Git configuration includes/driver sections are unsupported in engineering workspaces"
            );
        }
    }
    Ok(())
}
pub(super) trait Ownership: Sized {
    fn capture(path: &Path) -> Result<Self>;
    fn verify(&self) -> Result<()>;
}
impl Ownership for GitOwnership {
    fn capture(path: &Path) -> Result<Self> {
        let worktree = std::fs::canonicalize(path)?;
        let pointer = worktree.join(".git");
        let metadata = std::fs::symlink_metadata(&pointer)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "Git ownership pointer is a symlink"
        );
        let linked = metadata.is_file();
        let git_dir = if linked {
            let bytes = regular(&pointer)?.context("missing Git pointer")?;
            let value = std::str::from_utf8(&bytes)?
                .trim()
                .strip_prefix("gitdir: ")
                .context("invalid Git pointer")?;
            resolve(&worktree, value)?
        } else {
            ensure!(metadata.is_dir(), "invalid Git directory");
            std::fs::canonicalize(&pointer)?
        };
        let common_dir = if let Some(bytes) = regular(&git_dir.join("commondir"))? {
            resolve(&git_dir, std::str::from_utf8(&bytes)?)?
        } else {
            git_dir.clone()
        };
        if linked {
            let back = regular(&git_dir.join("gitdir"))?.context("missing Git back-reference")?;
            ensure!(
                resolve(&git_dir, std::str::from_utf8(&back)?)? == pointer,
                "Git back-reference does not own this worktree"
            );
        }
        let mut files = vec![
            git_dir.join("commondir"),
            git_dir.join("gitdir"),
            git_dir.join("config.worktree"),
            common_dir.join("config"),
            common_dir.join("info/attributes"),
            common_dir.join("info/exclude"),
            git_dir.join("info/exclude"),
            common_dir.join("info/grafts"),
            common_dir.join("objects/info/alternates"),
            common_dir.join("objects/info/http-alternates"),
        ];
        if linked {
            files.push(pointer);
        }
        safe_config(&common_dir.join("config"))?;
        safe_config(&git_dir.join("config.worktree"))?;
        // Alternate object stores/grafts can redirect the commit graph outside this ownership boundary.
        for file in [
            "objects/info/alternates",
            "objects/info/http-alternates",
            "info/grafts",
        ] {
            ensure!(
                regular(&common_dir.join(file))?.is_none(),
                "Git alternate stores/grafts are unsupported"
            );
        }
        let metadata = files
            .into_iter()
            .map(|p| Ok((p.clone(), hash(&p)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let owned = Self {
            worktree,
            git_dir,
            common_dir,
            metadata,
        };
        owned.verify()?;
        Ok(owned)
    }
    fn verify(&self) -> Result<()> {
        let check = || -> Result<()> {
            for path in [
                &self.worktree,
                &self.git_dir,
                &self.common_dir,
                &self.common_dir.join("objects"),
            ] {
                ensure!(
                    std::fs::canonicalize(path)? == *path
                        && std::fs::symlink_metadata(path)?.is_dir(),
                    "Git ownership directory changed"
                );
            }
            // HEAD/index are legitimately rewritten by harness Git operations, but
            // their filesystem ownership must never become a symlink or special file.
            regular(&self.git_dir.join("HEAD"))?.context("missing Git HEAD")?;
            regular(&self.git_dir.join("index"))?;
            for (path, expected) in &self.metadata {
                ensure!(
                    &hash(path)? == expected,
                    "Git ownership/configuration metadata changed: {}",
                    path.display()
                );
            }
            Ok(())
        };
        check().map_err(|e| {
            AgentFailure {
                kind: "git_ownership_changed".into(),
                detail: format!("{e:#}"),
                blocked: true,
            }
            .into()
        })
    }
}

#[cfg(test)]
mod config_discovery_tests {
    use super::*;
    #[test]
    fn partial_clone_rejection_is_local_and_never_echoes_config_subsections() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let config = tmp.path().join("config");
        for content in [
            "[extensions]\npartialClone = origin\n",
            "[remote \"https://credential-canary@example.invalid\"]\npromisor = true\n",
            "[remote \"origin\"]\npartialCloneFilter = blob:none\n",
        ] {
            std::fs::write(&config, content)?;
            let error =
                safe_config(&config).expect_err("partial clones must fail before Git probes");
            assert!(error.to_string().contains("partial-clone"));
            assert!(!error.to_string().contains("credential-canary"));
        }
        std::fs::write(
            &config,
            "[remote \"origin\"]\nurl = https://credential-canary@example.invalid/repo\n",
        )?;
        safe_config(&config)?;
        Ok(())
    }
}
