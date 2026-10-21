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
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
#[derive(Default)]
struct Fixture {
    paused: AtomicBool,
    calls: Mutex<HashMap<String, usize>>,
}
impl AgentRuntime for Fixture {
    fn prepare<'a>(
        &'a self,
        r: &'a AgentRequest,
        _: &'a ProcessRunner,
        _: &'a CancellationToken,
    ) -> AgentFuture<'a> {
        Box::pin(async move {
            // Optional test-only pacing leaves live states visible for native UI QA.
            // Keep the command numeric and bounded; production agents are unchanged.
            let delay = std::env::var("AUTONO_FIXTURE_DELAY_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| (1..=60).contains(value))
                .unwrap_or(2);
            let single_task = matches!(
                std::env::var("AUTONO_FIXTURE_SCENARIO").as_deref(),
                Ok("verification-failure" | "verification-timeout")
            );
            let observable =
                std::env::var("AUTONO_FIXTURE_SCENARIO").as_deref() == Ok("observability");
            let mut before = format!("sleep {delay}; ");
            let value = match r.role {
                AgentRole::Planner => {
                    let tasks = if single_task {
                        vec![("A", vec![])]
                    } else {
                        vec![
                            ("A", vec![]),
                            ("B", vec!["A"]),
                            ("C", vec![]),
                            ("D", vec!["B"]),
                            ("E", vec!["C"]),
                        ]
                    };
                    let requirements = if single_task {
                        json!([])
                    } else {
                        json!([{"kind":"product_decision","reason":"The fixture specification leaves the export format open","requested_action":"Choose the report export format","affected_tasks":["B"],"blocking":true,"evidence":"CSV or JSON is not specified","confidence":100,"permission":null}])
                    };
                    let milestones = if single_task {
                        json!([{"id":"m1","goal":"Verification failure fixture","task_ids":["A"]}])
                    } else {
                        json!([{"id":"m1","goal":"Foundation and parallel checks","task_ids":["A","C","E"]},{"id":"m2","goal":"Report and final assembly","task_ids":["B","D"]}])
                    };
                    json!({"human_requirements":requirements,"requirements":[{"id":"r","text":"Deliver an independently verified local report"}],"milestones":milestones,"tasks":tasks.into_iter().map(|(id,deps)|json!({"id":id,"goal":match id {"A"=>"Prepare report foundation","B"=>"Implement report export","C"=>"Add validation helpers","D"=>"Assemble report workflow",_=>"Verify validation coverage"},"description":format!("Implement fixture task {id}"),"acceptance_criteria":[format!("Task {id} artifact exists")],"dependencies":deps,"requirement_ids":["r"],"verification_expectations":["Configured shell check passes"],"role":"coder"})).collect::<Vec<_>>()})
                }
                AgentRole::Coder => {
                    let id = r
                        .cwd
                        .file_name()
                        .and_then(|name| name.to_str())
                        .and_then(|name| name.strip_prefix("task-"))
                        .filter(|id| ["A", "B", "C", "D", "E"].contains(id))
                        .unwrap_or("A");
                    let call = {
                        let mut calls = self.calls.lock().expect("fixture call counter");
                        let count = calls
                            .entry(format!("coder:{id}:{}", r.cwd.display()))
                            .or_default();
                        *count += 1;
                        *count
                    };
                    if observable && id == "C" && call > 1 {
                        before.push_str("printf 'checked\\n' > C.checked; ");
                    }
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
                    let call = {
                        let mut calls = self.calls.lock().expect("fixture call counter");
                        // Review worktrees differ by cycle; use the task id for this single-run fixture.
                        let count = calls.entry(format!("reviewer:{id}")).or_default();
                        *count += 1;
                        *count
                    };
                    if observable && id == 'E' && call == 1 {
                        json!({"invalid_fixture_review":true})
                    } else if observable && id == 'A' && call == 1 {
                        json!({"accepted":false,"criteria":[{"criterion_id":"A-C1","criterion":"Task A artifact exists","satisfied":false,"evidence":"Fixture review requires the next coding attempt"}],"findings":[{"severity":"medium","file":"A.txt","line":1,"explanation":"Fixture report foundation needs an explicit header","recommended_fix":"Add the header in the next attempt","confidence":0.98}],"blocker":""})
                    } else {
                        let findings = if observable && id == 'A' {
                            json!([{"severity":"low","file":"A.txt","line":1,"explanation":"Consider a more descriptive fixture header","recommended_fix":"Optional naming improvement","confidence":0.9}])
                        } else {
                            json!([])
                        };
                        json!({"accepted":true,"criteria":[{"criterion_id":format!("{id}-C1"),"criterion":format!("Task {id} artifact exists"),"satisfied":true,"evidence":"Fixture artifact and deterministic check inspected"}],"findings":findings,"blocker":""})
                    }
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
    config.verification.timeout_seconds = 30;
    if std::env::var("AUTONO_FIXTURE_SCENARIO").as_deref() == Ok("observability") {
        config.verification.commands[0][2] = "printf 'fixture verification: inspected\\n'; test -f README.md && { test ! -f C.txt || test -f C.checked; }".into();
    }
    if std::env::var("AUTONO_FIXTURE_SCENARIO").as_deref() == Ok("verification-failure") {
        config.verification.commands[0][2] =
            "printf 'fixture verification: intentional failure\\n'; exit 1".into();
    }
    if std::env::var("AUTONO_FIXTURE_SCENARIO").as_deref() == Ok("verification-timeout") {
        config.verification.commands.push(vec![
            "sh".into(),
            "-c".into(),
            "printf 'fixture verification: waiting\\n'; sleep 60".into(),
        ]);
        config.verification.timeout_seconds = 30;
    }
    // Keep the bridge's read-only settings view identical to the injected daemon config.
    tokio::fs::write(paths.config(), toml::to_string(&config)?).await?;
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
