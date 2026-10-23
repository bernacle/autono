//! v2 finite discovery. Git supplies names; descriptor-relative reads supply
//! bounded UTF-8 content. A name in an inventory never grants write permission.
use super::{delegation::denied, process::ProcessRunner, *};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeSet, path::Path};
use tokio_util::sync::CancellationToken;

fn ordinary(path: &str) -> bool {
    if super::delegation::validate_file(path).is_err() {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    if lower.split('/').any(|part| {
        matches!(
            part,
            "ci" | "infra"
                | "infrastructure"
                | "deploy"
                | "deployment"
                | "scripts"
                | "node_modules"
                | "vendor"
                | "target"
                | "dist"
                | "build"
                | "config"
                | "configuration"
                | "settings"
                | "hooks"
                | "workflows"
        ) || part.contains("credential")
            || part.contains("secret")
            || part.contains("private-key")
            || part.contains("private_key")
    }) {
        return false;
    }
    let name = lower.rsplit('/').next().unwrap_or("");
    let stem = name.split('.').next().unwrap_or("");
    if name.contains(".config.")
        || name.contains(".conf.")
        || matches!(
            stem,
            "config"
                | "settings"
                | "ci"
                | "build"
                | "deploy"
                | "deployment"
                | "infrastructure"
                | "infra"
                | "release"
                | "publish"
                | "setup"
        )
    {
        return false;
    }
    if matches!(
        name,
        "package.json"
            | "package-lock.json"
            | "cargo.toml"
            | "cargo.lock"
            | "makefile"
            | "dockerfile"
            | "compose.yml"
            | "setup.py"
            | "conftest.py"
            | "id_rsa"
            | "id_ed25519"
    ) {
        return false;
    }
    // Deliberately source-only: configuration, lockfiles, keys and executables
    // without a source extension require another explicit workflow.
    matches!(
        name.rsplit('.').next(),
        Some(
            "rs" | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "py"
                | "go"
                | "java"
                | "kt"
                | "swift"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "cs"
                | "rb"
                | "php"
                | "sh"
                | "css"
                | "scss"
                | "html"
                | "vue"
                | "svelte"
                | "md"
                | "txt"
                | "csv"
        )
    )
}
fn beneath(path: &str, roots: &[String]) -> bool {
    roots
        .iter()
        .any(|root| path.strip_prefix(root).is_some_and(|s| s.starts_with('/')))
}
pub(super) fn validate(scope: &SourceScope) -> Result<()> {
    ensure!(scope.version == 1, "unsupported source profile version");
    for roots in [&scope.read_subtrees, &scope.write_subtrees] {
        ensure!(
            (1..=8).contains(&roots.len()),
            "authorize 1..8 source subtrees"
        );
        let mut unique = BTreeSet::new();
        for root in roots {
            ensure!(
                ordinary(&format!("{root}/source.rs")) && unique.insert(root),
                "invalid or protected source subtree: {root}"
            );
        }
    }
    ensure!(
        scope
            .write_subtrees
            .iter()
            .all(|root| scope.read_subtrees.contains(root) || beneath(root, &scope.read_subtrees)),
        "write subtrees require read authority"
    );
    Ok(())
}
pub(super) fn can_read(contract: &DelegationContract, path: &str) -> bool {
    match &contract.request.source_scope {
        Some(scope) => {
            contract.version == 2 && ordinary(path) && beneath(path, &scope.read_subtrees)
        }
        None => contract.version == 1 && contract.request.writable_files.iter().any(|p| p == path),
    }
}
pub(super) fn can_write(contract: &DelegationContract, path: &str) -> bool {
    match &contract.request.source_scope {
        Some(scope) => {
            contract.version == 2 && ordinary(path) && beneath(path, &scope.write_subtrees)
        }
        None => contract.version == 1 && contract.request.writable_files.iter().any(|p| p == path),
    }
}

/// Only immutable Git object names before consent; no checkout scripts, network,
/// provider probes or inference. Runtime inventory also includes staged new files.
pub(super) async fn inventory(
    runner: &ProcessRunner,
    cwd: &Path,
    base: Option<&str>,
    scope: &SourceScope,
    stop: &CancellationToken,
) -> Result<Vec<String>> {
    let raw = match base {
        Some(base) => {
            runner
                .git(cwd, &["ls-tree", "-r", "-z", "--full-tree", base], stop)
                .await?
        }
        None => {
            runner
                .git(cwd, &["ls-files", "--stage", "-z"], stop)
                .await?
        }
    };
    let mut names = BTreeSet::new();
    let mut count = 0;
    for entry in raw.split('\0').filter(|e| !e.is_empty()) {
        count += 1;
        ensure!(
            count <= 4096,
            denied("source discovery exceeds 4096 repository entries")
        );
        let (header, name) = entry.split_once('\t').context("invalid source inventory")?;
        ensure!(
            !header.starts_with("160000 "),
            denied("repositories with submodules are unsupported by this source/candidate profile")
        );
        if !ordinary(name) || !beneath(name, &scope.read_subtrees) {
            continue;
        }
        ensure!(
            header.starts_with("100644 ") || header.starts_with("100755 "),
            denied(format!(
                "source entry is not an ordinary regular file: {name}"
            ))
        );
        names.insert(name.to_owned());
        ensure!(
            names.len() <= 128,
            denied("source discovery exceeds 128 readable files; narrow the subtrees")
        );
    }
    Ok(names.into_iter().collect())
}

/// Final candidate audit is independent of model output and catches deletions,
/// mode changes, scope changes by checks, and changes inherited from dependencies.
pub(super) async fn audit(
    run: &EngineeringRun,
    runner: &ProcessRunner,
    cwd: &Path,
    tree: &str,
    stop: &CancellationToken,
) -> Result<()> {
    let Some(auth) = &run.delegation else {
        return Ok(());
    };
    if auth.contract.version != 2 {
        return Ok(());
    }
    let raw = runner
        .git(
            cwd,
            &[
                "diff",
                "--raw",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "-z",
                &run.base_commit,
                tree,
                "--",
            ],
            stop,
        )
        .await?;
    let fields: Vec<_> = raw.split('\0').filter(|s| !s.is_empty()).collect();
    ensure!(
        fields.len() % 2 == 0 && fields.len() <= 256,
        denied("candidate exceeds scoped file limit")
    );
    let mut total = 0;
    for pair in fields.as_chunks::<2>().0 {
        let header: Vec<_> = pair[0].split_whitespace().collect();
        ensure!(header.len() == 5, denied("invalid candidate inventory"));
        let path = pair[1];
        let added = header[4] == "A" && header[0] == ":000000" && header[1] == "100644";
        let modified = header[4] == "M" && header[0].strip_prefix(':') == Some(header[1]);
        ensure!(
            can_write(&auth.contract, path)
                && (modified
                    || (added
                        && auth
                            .contract
                            .request
                            .source_scope
                            .as_ref()
                            .is_some_and(|s| s.allow_create))),
            denied(format!(
                "missing source write authority or prohibited file operation: {path}"
            ))
        );
        let content =
            super::scoped_files::read(cwd, path)?.context("candidate source is missing")?;
        total += content.len();
        ensure!(
            total <= 1024 * 1024,
            denied("candidate source exceeds 1 MiB")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn authorized(root: &Path) -> Result<EngineeringRun> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../apps/desktop/src/test/fixtures/completed-journal.json"
        ))?;
        let mut run: EngineeringRun = serde_json::from_value(fixture["run"].clone())?;
        let now = chrono::Utc::now();
        run.delegation = Some(DelegationAuthorization {
            authorized_at: now,
            expires_at: now + chrono::Duration::seconds(120),
            contract: DelegationContract {
                version: 2,
                request: DelegationProposalRequest {
                    submission: PlanSubmission {
                        id: run.id,
                        repository: run.repository.clone(),
                        base: run.requested_base.clone(),
                        specification: run.specification.clone(),
                    },
                    source_scope: Some(SourceScope::default()),
                    writable_files: vec![],
                    max_runtime_seconds: 120,
                    local_integration: true,
                },
                repository_git: run.repository_git.clone().context("fixture ownership")?,
                repository_identity: Default::default(),
                base_commit: run.base_commit.clone(),
                worktree_root: root.into(),
                settings: run.settings.clone(),
                verification_sandbox: "fixture".into(),
                user_config_digest: None,
                limitations: vec![],
                source_inventory: vec![],
            },
        });
        Ok(run)
    }
    #[test]
    fn source_profiles_enforce_batch_scope_creation_and_untracked_collision() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = std::fs::canonicalize(tmp.path())?;
        let cwd = root.join("task");
        std::fs::create_dir_all(cwd.join("src"))?;
        let mut run = authorized(&root)?;
        let response = |paths: &[&str]| serde_json::json!({"summary":"Candidate","blocker":"","edits":paths.iter().map(|path| serde_json::json!({"path":path,"content":"candidate"})).collect::<Vec<_>>()});
        for path in [
            "README.md",
            "src/.env",
            "src/AGENTS.md",
            "src/package.json",
            "src/infra/config.sh",
            "src/../escape.rs",
            "../escape.rs",
        ] {
            assert!(
                super::super::delegation::apply_edits(
                    &run,
                    &cwd,
                    response(&["src/first.rs", path]),
                    Some(&[])
                )
                .is_err(),
                "{path}"
            );
            assert!(
                !cwd.join("src/first.rs").exists(),
                "the complete batch must be validated first"
            );
        }
        std::fs::write(cwd.join("src/user.rs"), "user work")?;
        assert!(
            super::super::delegation::apply_edits(
                &run,
                &cwd,
                response(&["src/user.rs"]),
                Some(&[])
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(cwd.join("src/user.rs"))?,
            "user work"
        );
        super::super::delegation::apply_edits(&run, &cwd, response(&["src/new.rs"]), Some(&[]))?;
        assert_eq!(
            std::fs::read_to_string(cwd.join("src/new.rs"))?,
            "candidate"
        );
        let contract = &mut run.delegation.as_mut().context("auth")?.contract;
        let scope = contract.request.source_scope.as_mut().context("scope")?;
        scope.write_subtrees = vec!["tests".into()];
        assert!(can_read(contract, "src/user.rs"));
        assert!(!can_write(contract, "src/user.rs"));
        assert!(
            super::super::delegation::apply_edits(
                &run,
                &cwd,
                response(&["src/user.rs"]),
                Some(&["src/user.rs".into()])
            )
            .is_err()
        );
        let scope = run
            .delegation
            .as_mut()
            .context("auth")?
            .contract
            .request
            .source_scope
            .as_mut()
            .context("scope")?;
        scope.allow_create = false;
        assert!(
            super::super::delegation::apply_edits(
                &run,
                &cwd,
                response(&["tests/new.rs"]),
                Some(&[])
            )
            .is_err()
        );
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn nested_repositories_links_and_atomic_creation_cannot_overwrite_user_files() -> Result<()> {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir()?;
        let root = std::fs::canonicalize(tmp.path())?;
        let cwd = root.join("task");
        std::fs::create_dir_all(cwd.join("src/nested"))?;
        std::fs::write(cwd.join("src/nested/.git"), "gitdir: outside")?;
        assert!(super::super::scoped_files::read(&cwd, "src/nested/file.rs").is_err());
        assert!(super::super::scoped_files::create(&cwd, "src/nested/file.rs", "bad").is_err());
        std::fs::write(root.join("private"), "private")?;
        symlink(root.join("private"), cwd.join("src/link.rs"))?;
        assert!(super::super::scoped_files::read(&cwd, "src/link.rs").is_err());
        std::fs::hard_link(root.join("private"), cwd.join("src/hard.rs"))?;
        assert!(super::super::scoped_files::read(&cwd, "src/hard.rs").is_err());
        std::fs::write(cwd.join("src/user.rs"), "user")?;
        assert!(super::super::scoped_files::create(&cwd, "src/user.rs", "bad").is_err());
        assert_eq!(std::fs::read_to_string(cwd.join("src/user.rs"))?, "user");
        assert_eq!(std::fs::read_to_string(root.join("private"))?, "private");
        Ok(())
    }
    #[test]
    fn authority_is_explicit_and_excludes_protected_names_at_any_depth() {
        let s = SourceScope::default();
        assert!(validate(&s).is_ok());
        for path in [
            "../src/a.rs",
            "src/../a.rs",
            "src/.env",
            "src/AGENTS.md",
            "src/nested/CLAUDE.md",
            "src/package.json",
            "src/deploy/a.sh",
            "src/private_key.txt",
            "src/key.pem",
            "src/credentials.py",
            "src/vite.config.ts",
            "src/ci.ts",
            "src/build.sh",
            "src/config/routes.rs",
            "src/.git/config",
        ] {
            assert!(!ordinary(path), "{path}");
        }
        assert!(ordinary("src/reporting/export.ts"));
        for root in [".", "..", "src/../tests", "infra", ".github", "src//nested"] {
            let mut s = s.clone();
            s.write_subtrees = vec![root.into()];
            assert!(validate(&s).is_err(), "{root}");
        }
    }
}
