use std::{collections::BTreeMap, time::Duration};

use anyhow::Result;
use autono_core::{RunSpec, Store, Supervisor};
use autono_protocol::{Event, HostStatus, Run, RunStatus, Submission};
use reqwest::StatusCode;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Fixture {
    _dir: TempDir,
    base: String,
    token: String,
    client: reqwest::Client,
    stop: CancellationToken,
    store: Store,
    supervisor: Supervisor,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Fixture {
    async fn new() -> Result<Self> {
        Self::configured(8, 32, Duration::from_secs(5)).await
    }
    async fn configured(active: usize, pending: usize, body_timeout: Duration) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("runtime.sqlite3")).await?;
        let supervisor = Supervisor::with_limits(
            store.clone(),
            std::sync::Arc::new(autono_core::power::StayAwake::new(
                false,
                std::sync::Arc::new(autono_core::power::SystemPower),
            )),
            active,
            pending,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        let signal = stop.clone();
        let token = "test-secret-256-bit-equivalent-in-tests".to_owned();
        let app = autono_daemon::api::configured_router(
            supervisor.clone(),
            token.clone(),
            address,
            stop.clone(),
            body_timeout,
        );
        let server_supervisor = supervisor.clone();
        let task = tokio::spawn(async move {
            autono_daemon::server::serve(
                listener,
                app,
                server_supervisor,
                signal,
                Duration::from_millis(100),
                body_timeout,
            )
            .await
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            _dir: dir,
            base: format!("http://{address}"),
            token,
            client,
            stop,
            task,
            store,
            supervisor,
        })
    }

    fn get(&self, route: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}{route}", self.base))
            .bearer_auth(&self.token)
    }

    async fn finish(self) -> Result<()> {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), self.task).await???;
        Ok(())
    }
}

#[tokio::test]
async fn authentication_origin_host_and_validation_are_enforced() -> Result<()> {
    let fixture = Fixture::new().await?;
    assert_eq!(
        fixture
            .client
            .get(format!("{}/v1/status", fixture.base))
            .send()
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture
            .get("/v1/status")
            .header("Origin", "https://evil.example")
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture
            .get("/v1/status")
            .header("Host", "evil.example")
            .send()
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture
            .get("/v1/status")
            .header("Authorization", "Bearer wrong")
            .send()
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = fixture.get("/v1/status").send().await?;
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|s| s.to_str().ok()),
        Some("no-store")
    );
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    let status: HostStatus = response.error_for_status()?.json().await?;
    assert_eq!(status.database, "available");
    let invalid = RunSpec {
        command: Vec::new(),
        cwd: std::env::current_dir()?,
        env: BTreeMap::new(),
        timeout_seconds: None,
        keep_awake: false,
    };
    assert_eq!(
        fixture
            .client
            .post(format!("{}/v1/runs", fixture.base))
            .bearer_auth(&fixture.token)
            .json(&Submission {
                submission_key: uuid::Uuid::new_v4(),
                spec: invalid
            })
            .send()
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        fixture
            .get(&format!("/v1/runs/{}", uuid::Uuid::new_v4()))
            .send()
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    fixture.finish().await
}

#[cfg(unix)]
#[tokio::test]
async fn api_replays_durable_events_and_resumes_sse_after_cursor() -> Result<()> {
    let fixture = Fixture::new().await?;
    let spec = RunSpec {
        command: vec![
            "sh".into(),
            "-c".into(),
            "printf hello; printf error >&2".into(),
        ],
        cwd: std::env::current_dir()?,
        env: BTreeMap::new(),
        timeout_seconds: None,
        keep_awake: false,
    };
    let response = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&Submission {
            submission_key: uuid::Uuid::new_v4(),
            spec,
        })
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run: Run = response.json().await?;
    let stream = fixture
        .get(&format!("/v1/runs/{}/events/stream", run.id))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let streamed: Vec<Event> = stream
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert!(streamed.len() >= 5);
    let stored: Vec<Event> = fixture
        .get(&format!("/v1/runs/{}/events", run.id))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        serde_json::to_value(&stored)?,
        serde_json::to_value(&streamed)?
    );
    let cursor = stored[1].sequence;
    let replay = fixture
        .get(&format!("/v1/runs/{}/events/stream", run.id))
        .header("Last-Event-ID", cursor)
        .send()
        .await?
        .text()
        .await?;
    let replayed: Vec<Event> = replay
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(replayed.len(), stored.len() - 2);
    assert!(replayed.iter().all(|event| event.sequence > cursor));
    let finished: Run = fixture
        .get(&format!("/v1/runs/{}", run.id))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(finished.status, RunStatus::Succeeded);
    let runs: Vec<Run> = fixture
        .get("/v1/runs?limit=1&offset=0")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        fixture
            .client
            .post(format!("{}/v1/runs/{}/cancel", fixture.base, run.id))
            .bearer_auth(&fixture.token)
            .send()
            .await?
            .status(),
        StatusCode::CONFLICT
    );
    fixture.finish().await
}

#[cfg(unix)]
#[tokio::test]
async fn tcp_reset_during_commit_retry_is_idempotent_and_owned() -> Result<()> {
    use sqlx::Connection;
    use std::os::fd::AsRawFd;
    use tokio::io::AsyncWriteExt;
    let fixture = Fixture::configured(1, 1, Duration::from_secs(5)).await?;
    let mut spec = RunSpec {
        command: vec![
            "sh".into(),
            "-c".into(),
            "echo once >> effects; printf durable".into(),
        ],
        cwd: fixture._dir.path().to_owned(),
        env: BTreeMap::new(),
        timeout_seconds: None,
        keep_awake: false,
    };
    let submission = Submission {
        submission_key: uuid::Uuid::new_v4(),
        spec: spec.clone(),
    };
    let body = serde_json::to_string(&submission)?;
    let mut blocker = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(fixture._dir.path().join("runtime.sqlite3")),
    )
    .await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut blocker).await?;
    let authority = fixture.base.trim_start_matches("http://");
    let mut socket = tokio::net::TcpStream::connect(authority).await?;
    socket.write_all(format!("POST /v1/runs HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", fixture.token, body.len()).as_bytes()).await?;
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.supervisor.admission().active_runs != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    // SAFETY: linger is a valid socket option and socket remains owned here.
    anyhow::ensure!(
        unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                &linger as *const _ as *const libc::c_void,
                std::mem::size_of_val(&linger) as libc::socklen_t,
            )
        } == 0,
        "set TCP reset option"
    );
    drop(socket);
    let saturated = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&Submission {
            submission_key: uuid::Uuid::new_v4(),
            spec: spec.clone(),
        })
        .send()
        .await?;
    assert_eq!(saturated.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        saturated.json::<autono_protocol::ApiError>().await?.code,
        "overloaded"
    );
    let status =
        tokio::time::timeout(Duration::from_secs(1), fixture.get("/v1/status").send()).await??;
    assert!(status.status().is_success());
    sqlx::query("ROLLBACK").execute(&mut blocker).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.supervisor.admission().pending_submissions != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let accepted = fixture
        .store
        .find_submission(submission.submission_key)
        .await?
        .expect("reset submission must already be durable before retry");
    let retry: Run = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&submission)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(retry.id, accepted.id);
    fixture
        .get(&format!("/v1/runs/{}/events/stream", retry.id))
        .send()
        .await?
        .text()
        .await?;
    let existing: Run = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&submission)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(retry.id, existing.id);
    spec.command = vec!["false".into()];
    let conflict = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&Submission {
            submission_key: submission.submission_key,
            spec,
        })
        .send()
        .await?;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        tokio::fs::read_to_string(fixture._dir.path().join("effects")).await?,
        "once\n"
    );
    fixture.finish().await
}

#[cfg(unix)]
#[tokio::test]
async fn active_admission_saturation_keeps_status_and_cancel_available() -> Result<()> {
    let fixture = Fixture::configured(1, 4, Duration::from_secs(5)).await?;
    let input = RunSpec {
        command: vec![
            "sh".into(),
            "-c".into(),
            "echo ready; exec sleep 300".into(),
        ],
        cwd: std::env::current_dir()?,
        env: BTreeMap::new(),
        timeout_seconds: None,
        keep_awake: false,
    };
    let run = fixture.supervisor.submit(input.clone()).await?;
    let response = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&Submission {
            submission_key: uuid::Uuid::new_v4(),
            spec: input,
        })
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        fixture
            .get("/v1/status")
            .send()
            .await?
            .status()
            .is_success()
    );
    assert!(
        fixture
            .client
            .post(format!("{}/v1/runs/{}/cancel", fixture.base, run.id))
            .bearer_auth(&fixture.token)
            .send()
            .await?
            .status()
            .is_success()
    );
    fixture
        .get(&format!("/v1/runs/{}/events/stream", run.id))
        .send()
        .await?
        .text()
        .await?;
    assert_eq!(
        fixture.store.get(run.id).await?.status,
        RunStatus::Cancelled
    );
    fixture.finish().await
}

#[tokio::test]
async fn incomplete_authenticated_body_times_out_and_cannot_hold_shutdown() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = Fixture::configured(1, 1, Duration::from_millis(200)).await?;
    let authority = fixture.base.trim_start_matches("http://");
    let mut socket = tokio::net::TcpStream::connect(authority).await?;
    socket.write_all(format!("POST /v1/runs HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n{{", fixture.token).as_bytes()).await?;
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut reply)).await??;
    assert!(String::from_utf8(reply)?.contains("408 Request Timeout"));
    let mut socket = tokio::net::TcpStream::connect(authority).await?;
    socket.write_all(format!("POST /v1/runs HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {}\r\nContent-Length: 10000\r\n\r\n{{", fixture.token).as_bytes()).await?;
    fixture.finish().await
}

#[cfg(unix)]
#[tokio::test]
async fn slow_streams_and_incomplete_posts_leave_http_control_capacity() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = Fixture::configured(1, 4, Duration::from_secs(10)).await?;
    let input = RunSpec {
        command: vec![
            "sh".into(),
            "-c".into(),
            "echo ready; exec sleep 300".into(),
        ],
        cwd: std::env::current_dir()?,
        env: BTreeMap::new(),
        timeout_seconds: None,
        keep_awake: false,
    };
    let run = fixture.supervisor.submit(input).await?;
    let mut streams = Vec::new();
    for _ in 0..32 {
        streams.push(
            fixture
                .get(&format!("/v1/runs/{}/events/stream", run.id))
                .send()
                .await?
                .error_for_status()?,
        );
    }
    assert_eq!(
        fixture
            .get(&format!("/v1/runs/{}/events/stream", run.id))
            .send()
            .await?
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let authority = fixture.base.trim_start_matches("http://");
    let mut incomplete = Vec::new();
    for _ in 0..32 {
        let mut socket = tokio::net::TcpStream::connect(authority).await?;
        socket.write_all(format!("POST /v1/runs HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nExpect: 100-continue\r\n\r\n", fixture.token).as_bytes()).await?;
        // Hyper sends 100 Continue only once the authenticated body reader polls.
        // This is a deterministic barrier for occupying each body slot.
        let mut acknowledgment = [0u8; 25];
        tokio::time::timeout(
            Duration::from_secs(2),
            socket.read_exact(&mut acknowledgment),
        )
        .await??;
        assert_eq!(&acknowledgment, b"HTTP/1.1 100 Continue\r\n\r\n");
        incomplete.push(socket);
    }
    let response = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&Submission {
            submission_key: uuid::Uuid::new_v4(),
            spec: run.spec.clone(),
        })
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        response
            .json::<autono_protocol::ApiError>()
            .await?
            .error
            .contains("HTTP submission")
    );
    assert!(
        fixture
            .get("/v1/status")
            .send()
            .await?
            .status()
            .is_success()
    );
    assert!(
        fixture
            .client
            .post(format!("{}/v1/runs/{}/cancel", fixture.base, run.id))
            .bearer_auth(&fixture.token)
            .send()
            .await?
            .status()
            .is_success()
    );
    fixture.finish().await?;
    drop((streams, incomplete));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn status_exposes_finalization_degradation_and_idempotent_retry_keeps_the_owner() -> Result<()>
{
    use autono_protocol::{EventKind, RunPhase, RunView};
    use sqlx::Connection;
    use tokio::io::AsyncWriteExt;
    let fixture = Fixture::configured(1, 4, Duration::from_secs(5)).await?;
    let gate = fixture._dir.path().join("finalization-gate");
    let path = std::ffi::CString::new(gate.as_os_str().as_encoded_bytes())?;
    // SAFETY: valid path inside this test's temporary directory.
    anyhow::ensure!(
        unsafe { libc::mkfifo(path.as_ptr(), 0o600) } == 0,
        "create FIFO"
    );
    let submission = Submission {
        submission_key: uuid::Uuid::new_v4(),
        spec: RunSpec {
            command: vec![
                "sh".into(),
                "-c".into(),
                "echo ready; read go < finalization-gate; printf error >&2; exit 42".into(),
            ],
            cwd: fixture._dir.path().to_owned(),
            env: BTreeMap::new(),
            timeout_seconds: None,
            keep_awake: false,
        },
    };
    let run = fixture.supervisor.submit_keyed(submission.clone()).await?;
    let mut changed = fixture.store.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if fixture
                .store
                .events(run.id, 0, 500)
                .await?
                .iter()
                .any(|event| matches!(event.kind, EventKind::Output { .. }))
            {
                return Ok::<_, anyhow::Error>(());
            }
            changed.changed().await?;
        }
    })
    .await??;
    let mut lock = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(fixture._dir.path().join("runtime.sqlite3")),
    )
    .await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut lock).await?;
    let mut gate = tokio::fs::OpenOptions::new().write(true).open(gate).await?;
    gate.write_all(b"go\n").await?;
    drop(gate);
    let mut changed = fixture.supervisor.subscribe();
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            if fixture
                .supervisor
                .ownership(run.id)
                .is_some_and(|owner| owner.failed_attempts > 0)
            {
                break;
            }
            changed.changed().await.expect("supervisor remains alive");
        }
    })
    .await?;
    let status: HostStatus = fixture
        .get("/v1/status")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(!status.persistence.healthy);
    assert_eq!(
        (
            status.admission.active_runs,
            status.admission.executing_runs,
            status.admission.finalizing_runs
        ),
        (1, 0, 1)
    );
    let view: RunView = fixture
        .get(&format!("/v1/runs/{}", run.id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let owner = view.runtime.expect("live finalization ownership");
    assert_eq!(view.run.status, RunStatus::Running); // last committed state only
    assert_eq!(owner.phase, RunPhase::Finalizing);
    assert_eq!(
        owner.observed_result.expect("observed result").exit_code,
        Some(42)
    );
    let retry: RunView = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&submission)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(retry.run.id, run.id);
    assert_eq!(
        retry.runtime.expect("idempotent retry retains owner").phase,
        RunPhase::Finalizing
    );
    let new = fixture
        .client
        .post(format!("{}/v1/runs", fixture.base))
        .bearer_auth(&fixture.token)
        .json(&Submission {
            submission_key: uuid::Uuid::new_v4(),
            spec: submission.spec,
        })
        .send()
        .await?;
    assert_eq!(new.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        new.json::<autono_protocol::ApiError>().await?.code,
        "persistence_degraded"
    );
    assert!(
        fixture
            .client
            .post(format!("{}/v1/runs/{}/cancel", fixture.base, run.id))
            .bearer_auth(&fixture.token)
            .send()
            .await?
            .status()
            .is_success()
    );
    sqlx::query("ROLLBACK").execute(&mut lock).await?;
    fixture.finish().await
}
