#![cfg(unix)]
use anyhow::Result;
use autono_core::{
    Store, Supervisor,
    config::{Config, Paths},
    engineering::EngineeringService,
};
use autono_desktop::bridge::{Action, Bridge, Request};
use autono_protocol::{API_VERSION, DaemonMetadata, RunSpec, Submission};
use serde_json::json;
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
#[tokio::test]
async fn authenticated_bridge_projects_state_and_replays_bounded_sanitized_logs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let paths = Paths {
        home: dir.path().join("home"),
    };
    paths.prepare().await?;
    let store = Store::open(&paths.database()).await?;
    let supervisor = Supervisor::new(store, false);
    let config = Config::default();
    let service = EngineeringService::new(supervisor.clone(), paths.home.clone(), config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let token = "desktop-only-authentication-token-1234567890";
    let metadata = DaemonMetadata {
        api_version: API_VERSION,
        pid: std::process::id(),
        address,
        token: token.into(),
    };
    tokio::fs::write(paths.metadata(), serde_json::to_vec(&metadata)?).await?;
    tokio::fs::set_permissions(paths.metadata(), std::fs::Permissions::from_mode(0o600)).await?;
    let stop = CancellationToken::new();
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
        supervisor.clone(),
        stop.clone(),
        std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(5),
    ));
    let bridge = Bridge::connect(paths.clone()).map_err(|e| anyhow::anyhow!(e.message))?;
    let status = bridge
        .dispatch(Request::Status)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    assert_eq!(status["host"]["pid"], std::process::id());
    assert!(!status.to_string().contains(token));
    let raw = reqwest::Client::new()
        .get(format!("http://{address}/v1/status"))
        .send()
        .await?;
    assert_eq!(raw.status(), 401);
    let id = Uuid::new_v4();
    let process=supervisor.submit_keyed(Submission{submission_key:id,spec:RunSpec{command:vec!["sh".into(),"-c".into(),"printf 'API_KEY='; sleep 0.1; printf 'fixture-secret-value\\nverification passed\\nnot permitted\\n'".into()],cwd:dir.path().to_owned(),env:BTreeMap::from([("PRIVATE_PASSWORD".into(),"never-to-javascript".into()),("NO_COLOR".into(),"no".into())]),timeout_seconds:Some(5),keep_awake:false}}).await?;
    let mut cursor = 0;
    let mut lines = vec![];
    for _ in 0..50 {
        let page = bridge
            .dispatch(Request::ProcessEvents {
                id: process.id,
                after: cursor,
            })
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        let next = page["after"].as_i64().expect("cursor");
        if next > cursor {
            let replay = bridge
                .dispatch(Request::ProcessEvents {
                    id: process.id,
                    after: cursor,
                })
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            assert_eq!(replay, page);
        }
        cursor = next;
        lines.extend(page["lines"].as_array().expect("lines").clone());
        if lines
            .iter()
            .any(|l| l["text"].as_str().is_some_and(|t| t.contains("SUCCEEDED")))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    let text = json!(lines).to_string();
    assert!(text.contains("verification passed"));
    assert!(text.contains("not permitted"));
    assert!(!text.contains("fixture-secret-value"));
    assert!(cursor > 0);
    let processes = bridge
        .dispatch(Request::Processes { offset: 0 })
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    assert!(!processes.to_string().contains("never-to-javascript"));
    assert!(processes[0].get("spec").is_none());
    assert!(
        bridge
            .dispatch(Request::Action {
                id: Uuid::new_v4(),
                revision: 0,
                action: Action::Execute
            })
            .await
            .is_err()
    );
    let mut foreign = metadata.clone();
    foreign.address = "192.0.2.1:9999".parse()?;
    tokio::fs::write(paths.metadata(), serde_json::to_vec(&foreign)?).await?;
    assert!(Bridge::connect(paths.clone()).is_err());
    tokio::fs::write(paths.metadata(), serde_json::to_vec(&metadata)?).await?;
    tokio::fs::set_permissions(paths.metadata(), std::fs::Permissions::from_mode(0o644)).await?;
    assert!(Bridge::connect(paths.clone()).is_err());
    tokio::fs::remove_file(paths.metadata()).await?;
    std::os::unix::fs::symlink(dir.path().join("missing"), paths.metadata())?;
    assert!(Bridge::connect(paths).is_err());
    stop.cancel();
    server.await??;
    service.signal_shutdown();
    service.shutdown().await?;
    Ok(())
}
