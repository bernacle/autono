//! Trusted context and explicit untrusted review material.
use super::{PlannedTask, process::ProcessRunner};
use anyhow::{Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(super) fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
pub(super) async fn context(
    runner: &ProcessRunner,
    repository: &Path,
    base: &str,
    stop: &CancellationToken,
) -> Result<String> {
    let mut text = String::new();
    for name in [
        "README.md",
        "AGENTS.md",
        "CLAUDE.md",
        "docs/architecture.md",
        "docs/runtime.md",
    ] {
        let tree = runner
            .git(repository, &["ls-tree", base, "--", name], stop)
            .await?;
        // A blob symlink contains only its link target; never follow it or any checkout path.
        if !tree.starts_with("100644 blob ") && !tree.starts_with("100755 blob ") {
            continue;
        }
        let content = runner
            .git(repository, &["show", &format!("{base}:{name}")], stop)
            .await?;
        text.push_str(&format!(
            "\n{name}:\n{}\n",
            content.chars().take(6000).collect::<String>()
        ));
    }
    Ok(text)
}
#[derive(Serialize)]
pub(super) struct Change {
    pub status: String,
    pub path: String,
    pub instruction_configuration: bool,
    pub additional_scrutiny: bool,
}
pub(super) fn instruction_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or("");
    lower.starts_with(".github/instructions/")
        || lower.starts_with(".github/agents/")
        || matches!(
            name,
            "agents.md"
                | "agents.override.md"
                | "copilot-instructions.md"
                | "codex.md"
                | "claude.md"
                | "gemini.md"
                | "autono.toml"
                | ".mcp.json"
                | ".cursorrules"
                | ".claude.json"
        )
        || lower.split('/').any(|p| {
            matches!(
                p,
                ".claude" | ".codex" | ".agents" | ".cursor" | ".continue" | ".windsurf"
            )
        })
}
fn scrutinize(path: &str, commands: &[Vec<String>]) -> bool {
    if commands.iter().flatten().any(|arg| arg.contains(path)) {
        return true;
    }
    let path = path.to_ascii_lowercase();
    instruction_file(&path)
        || path.contains("test")
        || path.contains("check")
        || path.contains("verify")
        || path.contains("verification")
        || path.contains("build")
        || path.ends_with(".toml")
        || path.ends_with(".lock")
        || path.ends_with(".json")
        || path.ends_with(".yaml")
        || path.ends_with(".yml")
        || matches!(
            path.rsplit('/').next().unwrap_or(""),
            "cmakelists.txt"
                | "go.mod"
                | "go.sum"
                | "setup.py"
                | "setup.cfg"
                | "pipfile"
                | "justfile"
                | "tox.ini"
                | "pytest.ini"
        )
        || path.contains("requirements")
        || path.ends_with("makefile")
        || path.ends_with(".gitattributes")
        || path.ends_with(".gitmodules")
        || path.ends_with(".gitignore")
}
pub(super) async fn inventory(
    runner: &ProcessRunner,
    cwd: &Path,
    base: &str,
    commands: &[Vec<String>],
    stop: &CancellationToken,
) -> Result<Vec<Change>> {
    // No rename heuristics: both removed and added paths remain visible and flagged.
    let raw = runner
        .git(
            cwd,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--cached",
                "--no-renames",
                "--name-status",
                "-z",
                base,
                "--",
            ],
            stop,
        )
        .await?;
    let fields: Vec<_> = raw.split('\0').filter(|s| !s.is_empty()).collect();
    ensure!(fields.len() % 2 == 0, "invalid changed-file inventory");
    Ok(fields
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| Change {
            status: p[0].into(),
            path: p[1].into(),
            instruction_configuration: instruction_file(p[1]),
            additional_scrutiny: scrutinize(p[1], commands),
        })
        .collect())
}
pub(super) fn criteria(task: &PlannedTask) -> serde_json::Value {
    serde_json::Value::Array(
        task.criterion_ids()
            .iter()
            .zip(&task.acceptance_criteria)
            .map(|(id, text)| serde_json::json!({"id":id,"text":text}))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn flags_agent_configuration_and_custom_verification_entrypoints() {
        for path in [
            "nested/AGENTS.md",
            "AGENTS.override.md",
            ".codex/config.toml",
            ".github/instructions/review.md",
        ] {
            assert!(super::instruction_file(path));
        }
        assert!(!super::instruction_file(".github/workflows/ci.yml"));
        let commands = vec![vec!["sh".into(), "qualitygate".into()]];
        assert!(super::scrutinize("qualitygate", &commands));
        assert!(super::scrutinize("CMakeLists.txt", &commands));
        assert!(!super::scrutinize("src/feature.rs", &commands));
    }
}
