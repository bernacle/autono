//! Native fixed-route bridge against the authenticated production daemon/router.
#![cfg(any(target_os = "macos", target_os = "linux"))]
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
use autono_desktop::bridge::{Bridge, Request};
use autono_protocol::{API_VERSION, DaemonMetadata};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
struct Fixture(AtomicUsize);
impl AgentRuntime for Fixture {
    fn prepare<'a>(
        &'a self,
        r: &'a AgentRequest,
        _: &'a ProcessRunner,
        _: &'a CancellationToken,
    ) -> AgentFuture<'a> {
        Box::pin(async move {
            ensure!(
                r.bounded
                    && r.prompt.contains("src/report.sh")
                    && r.prompt.contains("reporting-seed"),
                "missing discovered source context"
            );
            let value = match r.role {
                AgentRole::Planner => {
                    self.0.fetch_add(1, Ordering::SeqCst);
                    json!({"decisions":[],"human_requirements":[],"requirements":[{"id":"r","text":"Export CSV"}],"milestones":[{"id":"m","goal":"CSV","task_ids":["export"]}],"tasks":[{"id":"export","goal":"CSV export","description":"Implement export","acceptance_criteria":["CSV output"],"dependencies":[],"requirement_ids":["r"],"verification_expectations":["Configured checks"],"role":"coder"}]})
                }
                AgentRole::Coder => {
                    json!({"decisions":[],"summary":"CSV export","blocker":"","edits":[{"path":"src/export.sh","content":"/bin/echo CSV\n"}]})
                }
                AgentRole::Reviewer => {
                    ensure!(
                        r.prompt.contains("diff_excerpt"),
                        "missing full review diff"
                    );
                    json!({"accepted":true,"criteria":[{"criterion_id":"export-C1","criterion":"CSV output","satisfied":true,"evidence":"Reviewed complete diff and check receipts"}],"findings":[],"blocker":""})
                }
            };
            let event = json!({"type":"result","subtype":"success","session_id":Uuid::new_v4(),"structured_output":value}).to_string();
            Ok(vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("printf '%s\\n' '{}'", event.replace('\'', "'\\''")),
            ])
        })
    }
    fn decode(&self, provider: Provider, output: &CommandOutput) -> Result<AgentResponse> {
        CliAgentRuntime.decode(provider, output)
    }
}
#[tokio::test]
async fn native_bridge_authorizes_exact_display_once_and_daemon_completes_after_client_drop()
-> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir()?;
    let root = std::fs::canonicalize(tmp.path())?;
    let paths = Paths {
        home: root.join("home"),
    };
    paths.prepare().await?;
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("src"))?;
    std::fs::write(repo.join("src/report.sh"), "# reporting-seed\n")?;
    let config = {
        let mut c = Config::default();
        c.agents.coder.provider = Provider::Claude;
        c.verification.commands = vec![vec![
            "/bin/sh".into(),
            "-c".into(),
            "if test -f src/export.sh; then test \"$(/bin/sh src/export.sh)\" = CSV; fi".into(),
        ]];
        c
    };
    let supervisor = Supervisor::new(Store::open(&paths.database()).await?, false);
    let runner = ProcessRunner::new(supervisor.clone());
    let stop = CancellationToken::new();
    runner.git(&repo, &["init", "-b", "main"], &stop).await?;
    runner.git(&repo, &["add", "."], &stop).await?;
    runner.git(&repo, &["commit", "-m", "seed"], &stop).await?;
    let fixture = Arc::new(Fixture(AtomicUsize::new(0)));
    let service = EngineeringService::with_agents(
        supervisor.clone(),
        paths.home.clone(),
        config,
        fixture.clone(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let token = "bridge-build-fixture-token-1234567890";
    let metadata = DaemonMetadata {
        api_version: API_VERSION,
        pid: std::process::id(),
        address,
        token: token.into(),
    };
    std::fs::write(paths.metadata(), serde_json::to_vec(&metadata)?)?;
    std::fs::set_permissions(paths.metadata(), std::fs::Permissions::from_mode(0o600))?;
    let app = autono_daemon::api::workflow_router(
        supervisor.clone(),
        service.clone(),
        token.into(),
        address,
        stop.clone(),
        Duration::from_secs(30),
    );
    let server = tokio::spawn(autono_daemon::server::serve(
        listener,
        app,
        supervisor.clone(),
        stop.clone(),
        Duration::from_secs(2),
        Duration::from_secs(30),
    ));
    let bridge = Bridge::connect(paths.clone()).map_err(|e| anyhow::anyhow!(e.message))?;
    let id = Uuid::new_v4();
    let contract = bridge
        .dispatch(Request::ProposeDelegation {
            request: DelegationProposalRequest {
                submission: PlanSubmission {
                    id,
                    repository: repo.clone(),
                    base: "main".into(),
                    specification: "Export CSV".into(),
                },
                source_scope: Some(SourceScope::default()),
                writable_files: vec![],
                max_runtime_seconds: 120,
                local_integration: true,
            },
        })
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    assert_eq!(contract["version"], 2);
    assert_eq!(fixture.0.load(Ordering::SeqCst), 0);
    assert!(!contract.to_string().contains(token));
    let mut tamper = contract.clone();
    tamper["request"]["source_scope"]["write_subtrees"] = json!(["infra"]);
    assert!(
        bridge
            .dispatch(Request::AuthorizeDelegation {
                id,
                displayed_contract: tamper
            })
            .await
            .is_err()
    );
    assert!(service.store().engineering_get(id).await.is_err());
    bridge
        .dispatch(Request::AuthorizeDelegation {
            id,
            displayed_contract: contract.clone(),
        })
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    drop(bridge); // the window/bridge cannot own or pause work
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut changes = service.store().subscribe();
        loop {
            let run = service.store().engineering_get(id).await?;
            if service.health().0.is_none() && !run.status.active() {
                return Ok::<_, anyhow::Error>(run);
            }
            changes.changed().await?;
        }
    })
    .await??;
    let run = service.store().engineering_get(id).await?;
    assert_eq!(
        run.status,
        EngineeringStatus::Completed,
        "{} {:?}",
        run.summary,
        run.human_actions
    );
    assert!(
        run.handoff
            .as_ref()
            .is_some_and(|h| h.completion_requirements_satisfied)
    );
    let reopened = Bridge::connect(paths).map_err(|e| anyhow::anyhow!(e.message))?;
    let restored = reopened
        .dispatch(Request::AuthorizeDelegation {
            id,
            displayed_contract: contract,
        })
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    assert_eq!(restored["status"], "COMPLETED");
    assert_eq!(fixture.0.load(Ordering::SeqCst), 1);
    assert!(!repo.join("src/export.sh").exists());
    service.shutdown().await?;
    stop.cancel();
    server.await??;
    Ok(())
}
