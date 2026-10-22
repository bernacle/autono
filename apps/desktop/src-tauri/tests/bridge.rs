#![cfg(unix)]
use anyhow::Result;
use autono_core::{
    Store, Supervisor,
    config::{Config, Paths},
    engineering::EngineeringService,
};
use autono_desktop::bridge::{Action, Bridge, Request};
use autono_protocol::{
    API_VERSION, DaemonMetadata, RunSpec, Submission, engineering::ReviewResult,
};
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
    let credential = "never-to-javascript";
    let escaped_credential = "canary-quoted-\"-slash-\\-unicode-é";
    let object = json!({
        (credential): {"nested":[{(escaped_credential):credential}]},
        "safe":"preserved object evidence"
    })
    .to_string()
    .replace("never", "\\u006eever");
    let array = json!([{(escaped_credential):[credential]}, {"safe":"preserved array evidence"}])
        .to_string();
    // Shared with rendered-evidence tests: real authenticated output is sanitized
    // using the same recursive projection as engineering run snapshots.
    let blocker_cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../src/test/fixtures/review-blocker-cases.json"
    ))?;
    let reviews: Vec<ReviewResult> = blocker_cases.iter().map(|case| serde_json::from_value(json!({
        "accepted":true,"criteria":[{"criterion_id":"A-C1","criterion":"Safe evidence remains readable","satisfied":true,"evidence":"Inspected the artifact"}],
        "findings":[],"blocker":case["original"]
    }))).collect::<std::result::Result<_,_>>()?;
    for (review, case) in reviews.iter().zip(&blocker_cases) {
        assert_eq!(
            review.accepts(),
            case["accepts"].as_bool().expect("expected verdict")
        );
    }
    let process = supervisor.submit_keyed(Submission {
        submission_key: id,
        spec: RunSpec {
            command: vec!["sh".into(), "-c".into(), "printf 'API_KEY='; sleep 0.1; printf 'fixture-secret-value\\nverification passed\\nnot permitted\\n'; printf '%s\\n' \"$FIXTURE_OBJECT\" \"$FIXTURE_ARRAY\" \"$FIXTURE_REVIEWS\"".into()],
            cwd: dir.path().to_owned(),
            env: BTreeMap::from([
                ("PRIVATE_PASSWORD".into(), credential.into()),
                ("PRIVATE_TOKEN".into(), escaped_credential.into()),
                ("FIXTURE_OBJECT".into(), object),
                ("FIXTURE_ARRAY".into(), array),
                ("FIXTURE_REVIEWS".into(), json!({"review_regressions":reviews}).to_string()),
                ("NO_COLOR".into(), "no".into())
            ]),
            timeout_seconds: Some(5), keep_awake: false,
        }
    }).await?;
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
    assert!(!lines.is_empty());
    for line in &lines {
        let timestamp = line["timestamp"]
            .as_str()
            .expect("recorded event timestamp");
        chrono::DateTime::parse_from_rfc3339(timestamp)?;
    }
    let projected_reviews: Vec<ReviewResult> = lines
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line["text"].as_str()?).ok())
        .find_map(|value| serde_json::from_value(value.get("review_regressions")?.clone()).ok())
        .expect("authenticated projected review evidence");
    assert_eq!(projected_reviews.len(), reviews.len());
    for ((projected, original), case) in projected_reviews.iter().zip(&reviews).zip(&blocker_cases)
    {
        assert_eq!(
            projected.accepts(),
            original.accepts(),
            "verdict drift: {}",
            case["name"]
        );
        assert_eq!(
            projected.blocker,
            case["projected"].as_str().expect("expected projection")
        );
        assert!(
            projected
                .blocker
                .chars()
                .all(|c| !c.is_control() || c == '\n' || c == '\t')
        );
        assert!(!projected.blocker.contains(token));
        assert_eq!(projected.criteria[0].evidence, "Inspected the artifact");
    }
    for line in &lines {
        if let Ok(decoded) =
            serde_json::from_str::<serde_json::Value>(line["text"].as_str().unwrap_or(""))
        {
            assert_no_canary(&decoded, &[credential, escaped_credential]);
        }
    }
    let text = json!(lines).to_string();
    assert!(!text.contains("never-to-javascript"));
    assert!(text.contains("verification passed"));
    assert!(text.contains("preserved object evidence"));
    assert!(text.contains("preserved array evidence"));
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

fn assert_no_canary(value: &serde_json::Value, canaries: &[&str]) {
    match value {
        serde_json::Value::String(text) => assert!(canaries.iter().all(|s| !text.contains(s))),
        serde_json::Value::Array(values) => {
            values.iter().for_each(|v| assert_no_canary(v, canaries))
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                assert!(canaries.iter().all(|s| !key.contains(s)));
                assert_no_canary(value, canaries);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn authenticated_run_projection_preserves_nonempty_review_blockers() -> Result<()> {
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        routing::get,
    };
    let dir = tempfile::tempdir()?;
    let paths = Paths {
        home: dir.path().join("home"),
    };
    paths.prepare().await?;
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../src/test/fixtures/review-blocker-cases.json"
    ))?;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../src/test/fixtures/completed-journal.json"
    ))?;
    let mut run = fixture["run"].clone();
    let template = run["tasks"][0]["attempts"][0]["reviews"][0].clone();
    let mut records = Vec::new();
    for case in &cases {
        let mut record = template.clone();
        record["session"]["id"] = json!(Uuid::new_v4());
        record["review"]["accepted"] = json!(true);
        record["review"]["findings"] = json!([]);
        record["review"]["blocker"] = case["original"].clone();
        let review: ReviewResult = serde_json::from_value(record["review"].clone())?;
        assert_eq!(
            review.accepts(),
            case["accepts"].as_bool().expect("verdict")
        );
        records.push(record);
    }
    run["tasks"][0]["attempts"][0]["review"] = records[0]["review"].clone();
    run["tasks"][0]["attempts"][0]["reviews"] = json!(records);
    run["summary"] = json!("Readable run summary");
    run["decisions"] = json!([{"id":"D-csv","policy_basis":"Exact A2 file scope", "proposal":{"chosen":"CSV","rationale":"Existing convention; Bearer decision-canary-1234567890"},"audit":[{"kind":"review_rejected"}]}]);
    let id: Uuid = serde_json::from_value(run["id"].clone())?;
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
    // A token-gated loopback snapshot fixture exercises Request::Run separately
    // from the real daemon/process-event regression above. No agents execute.
    let app = Router::new().route(
        "/v1/engineering/{id}",
        get(move |headers: HeaderMap| {
            let run = run.clone();
            async move {
                if headers.get("authorization").and_then(|h| h.to_str().ok())
                    == Some(format!("Bearer {token}").as_str())
                {
                    (StatusCode::OK, Json(run))
                } else {
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error":"authentication required"})),
                    )
                }
            }
        }),
    );
    let stop = CancellationToken::new();
    let server_stop = stop.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
    });
    let unauthorized = reqwest::Client::new()
        .get(format!("http://{address}/v1/engineering/{id}"))
        .send()
        .await?;
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
    let bridge = Bridge::connect(paths).map_err(|e| anyhow::anyhow!(e.message))?;
    let projected = bridge
        .dispatch(Request::Run { id })
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    assert_eq!(projected["summary"], "Readable run summary");
    assert_eq!(projected["decisions"][0]["proposal"]["chosen"], "CSV");
    assert_eq!(
        projected["decisions"][0]["policy_basis"],
        "Exact A2 file scope"
    );
    assert_eq!(
        projected["decisions"][0]["audit"][0]["kind"],
        "review_rejected"
    );
    assert!(!projected.to_string().contains("decision-canary-1234567890"));
    assert!(!projected.to_string().contains(token));
    let attempt = &projected["tasks"][0]["attempts"][0];
    assert!(!serde_json::from_value::<ReviewResult>(attempt["review"].clone())?.accepts());
    let projected_records = attempt["reviews"].as_array().expect("review records");
    assert_eq!(projected_records.len(), cases.len());
    for (record, case) in projected_records.iter().zip(&cases) {
        let review: ReviewResult = serde_json::from_value(record["review"].clone())?;
        assert_eq!(
            review.accepts(),
            case["accepts"].as_bool().expect("verdict"),
            "{}",
            case["name"]
        );
        assert_eq!(
            review.blocker,
            case["projected"].as_str().expect("projected blocker")
        );
    }
    stop.cancel();
    server.await??;
    Ok(())
}
