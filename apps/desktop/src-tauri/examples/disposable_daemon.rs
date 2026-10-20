//! Manual native acceptance fixture: real daemon router, supervisor, Git and SQLite;
//! injected shell-only agents. Not part of the production application.
use anyhow::{Result, ensure};
use autono_core::{
    Store, Supervisor,
    config::{Config, Paths},
    engineering::{
        agent::{AgentFuture, AgentRequest, AgentResponse, AgentRuntime, CliAgentRuntime},
        process::{CommandOutput, ProcessRunner},
        *,
    },
};
use autono_protocol::{API_VERSION, DaemonMetadata};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
#[derive(Default)]
struct Fixture {
    paused: AtomicBool,
}
impl AgentRuntime for Fixture {
    fn prepare<'a>(
        &'a self,
        r: &'a AgentRequest,
        _: &'a ProcessRunner,
        _: &'a CancellationToken,
    ) -> AgentFuture<'a> {
        Box::pin(async move {
            let mut before = String::from("sleep 2; ");
            let value = match r.role {
                AgentRole::Planner => {
                    let tasks = vec![
                        ("A", vec![]),
                        ("B", vec!["A"]),
                        ("C", vec![]),
                        ("D", vec!["B"]),
                        ("E", vec!["C"]),
                    ];
                    json!({"human_requirements":[{"kind":"product_decision","reason":"The fixture specification leaves the export format open","requested_action":"Choose the report export format","affected_tasks":["B"],"blocking":true,"evidence":"CSV or JSON is not specified","confidence":100,"permission":null}],"requirements":[{"id":"r","text":"Deliver an independently verified local report"}],"milestones":[{"id":"m1","goal":"Foundation and parallel checks","task_ids":["A","C","E"]},{"id":"m2","goal":"Report and final assembly","task_ids":["B","D"]}],"tasks":tasks.into_iter().map(|(id,deps)|json!({"id":id,"goal":match id {"A"=>"Prepare report foundation","B"=>"Implement report export","C"=>"Add validation helpers","D"=>"Assemble report workflow",_=>"Verify validation coverage"},"description":format!("Implement fixture task {id}"),"acceptance_criteria":[format!("Task {id} artifact exists")],"dependencies":deps,"requirement_ids":["r"],"verification_expectations":["Configured shell check passes"],"role":"coder"})).collect::<Vec<_>>()})
                }
                AgentRole::Coder => {
                    let id = r
                        .prompt
                        .split("\"id\":\"")
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                        .unwrap_or("A");
                    let id = if ["A", "B", "C", "D", "E"].contains(&id) {
                        id
                    } else {
                        "A"
                    };
                    if id == "B" && !self.paused.swap(true, Ordering::SeqCst) {
                        json!({"summary":"Paused before producing the export","blocker":"Inspect fixture export destination; confirm that no external report was sent before continuing"})
                    } else {
                        before.push_str(&format!(
                            "printf '%s\\n' 'fixture implementation {id}' > {id}.txt; "
                        ));
                        json!({"summary":format!("Implemented {id}"),"blocker":""})
                    }
                }
                AgentRole::Reviewer => {
                    let id = r
                        .prompt
                        .split("-C1")
                        .next()
                        .and_then(|s| s.chars().last())
                        .filter(|c| "ABCDE".contains(*c))
                        .unwrap_or('A');
                    json!({"accepted":true,"criteria":[{"criterion_id":format!("{id}-C1"),"criterion":format!("Task {id} artifact exists"),"satisfied":true,"evidence":"Fixture artifact and deterministic check inspected"}],"findings":[],"blocker":""})
                }
            };
            let session = Uuid::new_v4();
            let events = match r.config.provider {
                Provider::Claude => format!(
                    "{}\n",
                    json!({"type":"result","subtype":"success","session_id":session,"structured_output":value})
                ),
                Provider::Codex => format!(
                    "{}\n{}\n{}\n",
                    json!({"type":"thread.started","thread_id":session}),
                    json!({"type":"item.completed","item":{"type":"agent_message","text":value.to_string()}}),
                    json!({"type":"turn.completed"})
                ),
            };
            before.push_str(&format!("printf '%s' '{}'", events.replace('\'', "'\\''")));
            Ok(vec!["sh".into(), "-c".into(), before])
        })
    }
    fn decode(&self, p: Provider, o: &CommandOutput) -> Result<AgentResponse> {
        CliAgentRuntime.decode(p, o)
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("supply a fresh absolute disposable home");
    ensure!(
        home.is_absolute() && !home.exists(),
        "use a fresh absolute home"
    );
    let paths = Paths { home };
    paths.prepare().await?;
    let repo = paths.home.join("fixture-repository");
    tokio::fs::create_dir(&repo).await?;
    let mut config = Config::default();
    config.runner.keep_awake = false;
    config.permissions.rules.insert(
        PermissionOperation::RunChecks,
        PermissionRule::RequireApproval,
    );
    config.verification.commands = vec![vec![
        "sh".into(),
        "-c".into(),
        "printf 'fixture verification: passed\\n'; test -f README.md".into(),
    ]];
    tokio::fs::write(paths.config(), toml_config()).await?;
    let store = Store::open(&paths.database()).await?;
    let supervisor = Supervisor::configured(store, &config.runner);
    let runner = ProcessRunner::new(supervisor.clone());
    let stop = CancellationToken::new();
    runner.git(&repo, &["init", "-b", "main"], &stop).await?;
    tokio::fs::write(
        repo.join("README.md"),
        "Disposable native acceptance fixture\n",
    )
    .await?;
    runner.git(&repo, &["add", "README.md"], &stop).await?;
    runner
        .git(&repo, &["commit", "-m", "fixture seed"], &stop)
        .await?;
    let service = EngineeringService::with_agents(
        supervisor.clone(),
        paths.home.clone(),
        config,
        Arc::new(Fixture::default()),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let token = format!("{}{}", Uuid::new_v4(), Uuid::new_v4());
    let metadata = DaemonMetadata {
        api_version: API_VERSION,
        pid: std::process::id(),
        address,
        token: token.clone(),
    };
    tokio::fs::write(paths.metadata(), serde_json::to_vec(&metadata)?).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(paths.metadata(), std::fs::Permissions::from_mode(0o600))
            .await?;
    }
    println!(
        "Disposable daemon ready: {address}\nAUTONO_HOME={}\nRepository: {}\nNo provider executables or inference APIs are used.",
        paths.home.display(),
        repo.display()
    );
    let app = autono_daemon::api::workflow_router(
        supervisor.clone(),
        service.clone(),
        token,
        address,
        stop.clone(),
        std::time::Duration::from_secs(5),
    );
    let signal = stop.clone();
    tokio::spawn(async move {
        let _ = autono_daemon::shutdown_signal().await;
        signal.cancel();
    });
    autono_daemon::server::serve(
        listener,
        app,
        supervisor,
        stop,
        std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(5),
    )
    .await?;
    service.signal_shutdown();
    service.shutdown().await?;
    tokio::fs::remove_file(paths.metadata()).await?;
    Ok(())
}
fn toml_config() -> &'static str {
    "[runner]\nkeep_awake = false\n[permissions.rules]\nrun_checks = 'REQUIRE_APPROVAL'\n[verification]\ncommands = [['sh', '-c', \"printf 'fixture verification: passed\\\\n'; test -f README.md\"]]\ntimeout_seconds = 30\n"
}
