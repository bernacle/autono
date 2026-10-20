#![cfg(unix)]
use anyhow::{Result, ensure};
use autono_core::{
    Store, Supervisor,
    config::Config,
    engineering::{
        agent::{AgentFuture, AgentRequest, AgentResponse, AgentRuntime, CliAgentRuntime},
        process::{CommandOutput, ProcessRunner},
        *,
    },
};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Fake;
impl AgentRuntime for Fake {
    fn prepare<'a>(
        &'a self,
        r: &'a AgentRequest,
        _: &'a ProcessRunner,
        _: &'a CancellationToken,
    ) -> AgentFuture<'a> {
        Box::pin(async move {
            let value = match r.role {
                AgentRole::Planner => {
                    json!({"human_requirements":[{"kind":"product_decision","reason":"Feature behavior needs a concrete developer choice","requested_action":"Choose the feature mode","affected_tasks":["t"],"blocking":true,"evidence":"Specification does not name the feature mode","confidence":90,"permission":null}],"requirements":[{"id":"r","text":"Feature"}],"milestones":[{"id":"m","goal":"Feature","task_ids":["t"]}],"tasks":[{"id":"t","goal":"Feature","description":"Implement feature","acceptance_criteria":["Feature exists"],"dependencies":[],"requirement_ids":["r"],"verification_expectations":["Check feature"],"role":"coder"}]})
                }
                AgentRole::Coder => json!({"summary":"Implemented feature","blocker":""}),
                AgentRole::Reviewer => {
                    json!({"accepted":true,"criteria":[{"criterion_id":"t-C1","criterion":"Feature exists","satisfied":true,"evidence":"Inspected feature.txt and the passing check"}],"findings":[],"blocker":""})
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
            let script = format!(
                "{}printf '%s' '{}'",
                if r.role == AgentRole::Coder {
                    "echo implementation > feature.txt; "
                } else {
                    ""
                },
                events.replace('\'', "'\\''")
            );
            Ok(vec!["sh".into(), "-c".into(), script])
        })
    }
    fn decode(&self, p: Provider, o: &CommandOutput) -> Result<AgentResponse> {
        CliAgentRuntime.decode(p, o)
    }
}
async fn inactive(service: &EngineeringService, id: Uuid) -> Result<EngineeringRun> {
    tokio::time::timeout(std::time::Duration::from_secs(15),async{let mut changed=service.store().subscribe();loop{let r=service.store().engineering_get(id).await?;if !r.status.active()&&service.health().0.is_none(){return Ok(r);}tokio::select!{_ = changed.changed()=>{},_ = tokio::time::sleep(std::time::Duration::from_millis(10))=>{}}}}).await?
}
#[tokio::test]
async fn authenticated_api_owns_planning_execution_evidence_and_integration() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = tokio::fs::canonicalize(dir.path()).await?;
    let repo = root.join("repo");
    tokio::fs::create_dir(&repo).await?;
    let store = Store::open(&root.join("db")).await?;
    let supervisor = Supervisor::new(store, false);
    let runner = ProcessRunner::new(supervisor.clone());
    let stop = CancellationToken::new();
    runner.git(&repo, &["init", "-b", "main"], &stop).await?;
    tokio::fs::write(repo.join("README.md"), "Feature seed").await?;
    runner.git(&repo, &["add", "."], &stop).await?;
    runner.git(&repo, &["commit", "-m", "seed"], &stop).await?;
    let mut config = Config::default();
    config.runner.keep_awake = false;
    config.verification.commands = vec![vec!["test".into(), "-f".into(), "feature.txt".into()]];
    let service = EngineeringService::with_agents(
        supervisor.clone(),
        root.join("home"),
        config,
        Arc::new(Fake),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let token = "engineering-api-test-token";
    let app = autono_daemon::api::workflow_router(
        supervisor.clone(),
        service.clone(),
        token.into(),
        address,
        stop.clone(),
        std::time::Duration::from_secs(5),
    );
    let server = tokio::spawn(autono_daemon::server::serve(
        listener,
        app,
        supervisor,
        stop.clone(),
        std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(5),
    ));
    let http = reqwest::Client::builder().no_proxy().build()?;
    let base = format!("http://{address}");
    assert_eq!(
        http.get(format!("{base}/v1/engineering"))
            .send()
            .await?
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let id = Uuid::new_v4();
    let request = PlanSubmission {
        id,
        repository: repo.clone(),
        base: "HEAD".into(),
        specification: "Implement feature".into(),
    };
    let response = http
        .post(format!("{base}/v1/engineering"))
        .bearer_auth(token)
        .json(&request)
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    drop(response);
    let run = inactive(&service, id).await?;
    assert_eq!(
        run.status,
        EngineeringStatus::AwaitingApproval,
        "{}",
        run.summary
    );
    let report: EngineeringRun = http
        .post(format!("{base}/v1/engineering/{id}/preflight"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    inactive(&service, id).await?;
    assert!(!report.preflight.expect("report").ready);
    let actions: Vec<HumanAction> = http
        .get(format!("{base}/v1/engineering/{id}/blockers"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(actions.len(), 1);
    assert_eq!(
        http.post(format!("{base}/v1/engineering/{id}/execute"))
            .bearer_auth(token)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    inactive(&service, id).await?;
    let request = ResolutionRequest {
        request_id: Uuid::new_v4(),
        disposition: ResolutionDisposition::Resolve,
        note: "Use the local feature mode".into(),
        effect: EffectState::NotPerformed,
        permission: None,
    };
    let route = format!(
        "{base}/v1/engineering/{id}/blockers/{}/resolve",
        actions[0].id
    );
    assert_eq!(
        http.post(&route).json(&request).send().await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let (first, duplicate) = tokio::join!(
        http.post(&route).bearer_auth(token).json(&request).send(),
        http.post(&route).bearer_auth(token).json(&request).send()
    );
    first?.error_for_status()?;
    duplicate?.error_for_status()?;
    assert_eq!(
        service.store().engineering_get(id).await?.human_actions[0]
            .audit
            .len(),
        2
    );
    let mut conflict = request.clone();
    conflict.note = "Different decision".into();
    assert_eq!(
        http.post(&route)
            .bearer_auth(token)
            .json(&conflict)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let response = http
        .post(format!("{base}/v1/engineering/{id}/execute"))
        .bearer_auth(token)
        .send()
        .await?;
    ensure!(
        response.status().is_success(),
        "execute failed: {}",
        response.text().await?
    );
    let run = inactive(&service, id).await?;
    assert_eq!(run.status, EngineeringStatus::Verified, "{}", run.summary);
    assert_eq!(
        http.post(format!("{base}/v1/engineering/{id}/execute"))
            .bearer_auth(token)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let process = run.tasks[0].attempts[0].verification[0].process_run_id;
    let events: Vec<autono_protocol::Event> = http
        .get(format!("{base}/v1/runs/{process}/events"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(!events.is_empty());
    http.post(format!("{base}/v1/engineering/{id}/integrate"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?;
    let run = inactive(&service, id).await?;
    assert_eq!(run.status, EngineeringStatus::Completed, "{}", run.summary);
    assert!(!repo.join("feature.txt").exists());
    let events: Vec<EngineeringEvent> = http
        .get(format!("{base}/v1/engineering/{id}/events"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(events.iter().any(|e| e.kind == "completed"));
    // Engineering cancellation must not consume the exhausted POST-admission budget.
    let next = Uuid::new_v4();
    let request = PlanSubmission {
        id: next,
        repository: repo,
        base: "HEAD".into(),
        specification: "Another feature".into(),
    };
    http.post(format!("{base}/v1/engineering"))
        .bearer_auth(token)
        .json(&request)
        .send()
        .await?
        .error_for_status()?;
    inactive(&service, next).await?;
    let sockets=tokio::time::timeout(std::time::Duration::from_secs(10),async {
        use tokio::io::{AsyncBufReadExt,AsyncWriteExt};
        let mut sockets=Vec::new();
        for _ in 0..32 {
            let socket=tokio::net::TcpStream::connect(address).await?;
            let mut socket=tokio::io::BufReader::new(socket);
            socket.get_mut().write_all(format!("POST /v1/engineering HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Length: 1024\r\nExpect: 100-continue\r\n\r\n").as_bytes()).await?;
            let mut line=String::new();socket.read_line(&mut line).await?;ensure!(line.contains("100 Continue"),"expected body admission: {line}");
            line.clear();socket.read_line(&mut line).await?;sockets.push(socket);
        }
        Ok::<_,anyhow::Error>(sockets)
    }).await??;
    assert!(
        http.post(format!("{base}/v1/engineering/{next}/cancel"))
            .bearer_auth(token)
            .send()
            .await?
            .status()
            .is_success()
    );
    assert_eq!(
        service.store().engineering_get(next).await?.status,
        EngineeringStatus::Cancelled
    );
    drop(sockets);
    service.signal_shutdown();
    stop.cancel();
    server.await??;
    service.shutdown().await?;
    Ok(())
}
