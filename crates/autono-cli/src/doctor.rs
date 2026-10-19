use std::{process::Stdio, time::Duration};

use anyhow::Result;
use autono_core::{
    Store,
    config::{Config, Paths},
    power,
};
use autono_protocol::HostStatus;
use tokio::process::Command;

use crate::client::Client;

pub async fn run(paths: &Paths) -> Result<()> {
    println!("Autono Runtime {}", env!("CARGO_PKG_VERSION"));
    println!(
        "platform: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("home: {}", paths.home.display());
    let (git, codex, claude, docker, gh) = tokio::join!(
        tool("git"),
        tool("codex"),
        tool("claude"),
        tool("docker"),
        tool("gh")
    );
    for (name, result) in [
        ("git", git),
        ("Codex CLI", codex),
        ("Claude CLI", claude),
        ("Docker", docker),
        ("GitHub CLI", gh),
    ] {
        println!("{name}: {result}");
    }
    println!("stay-awake: {}", power::capability().await);
    match Config::load(paths).await {
        Ok(config) => println!(
            "configuration: ok (keep_awake={})",
            config.runner.keep_awake
        ),
        Err(error) => println!("configuration: ERROR ({error:#})"),
    }
    match probe("git", &["rev-parse", "--show-toplevel"]).await {
        Ok(root) => {
            println!("repository: {root}");
            match probe("git", &["status", "--short", "--branch"]).await {
                Ok(status) => println!("repository status:\n{status}"),
                Err(error) => println!("repository status: {error:#}"),
            }
        }
        Err(_) => println!("repository: not inside a git repository (or git unavailable)"),
    }
    match Client::connect(paths).await {
        Ok(client) => match client.get::<HostStatus>("/v1/status").await {
            Ok(status) => println!(
                "daemon: reachable (pid {}, database {}, persistence healthy={}, finalizing={}, inhibition held={})",
                status.pid,
                status.database,
                status.persistence.healthy,
                status.admission.finalizing_runs,
                status.power.held
            ),
            Err(error) => println!("daemon: unavailable ({error:#})"),
        },
        Err(error) => println!("daemon: unavailable ({error:#})"),
    }
    if tokio::fs::try_exists(paths.database()).await? {
        match Store::inspect(&paths.database()).await {
            Ok(store) => match tokio::time::timeout(Duration::from_secs(5), store.health()).await {
                Ok(Ok(())) => println!("database: healthy (SQLite quick_check)"),
                Ok(Err(error)) => println!("database: ERROR ({error:#})"),
                Err(_) => println!("database: health inspection timed out after 5 seconds"),
            },
            Err(error) => println!("database: ERROR ({error:#})"),
        }
    } else {
        println!("database: not initialized (start autono daemon)");
    }
    Ok(())
}

async fn tool(name: &str) -> String {
    match probe(name, &["--version"]).await {
        Ok(version) => format!(
            "available ({})",
            version.lines().next().unwrap_or("no version output")
        ),
        Err(error) => format!("unavailable ({error:#}); optional diagnostic only"),
    }
}

async fn probe(program: &str, args: &[&str]) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(3), command.output()).await??;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
