use anyhow::{Context, Result};
use autono_core::config::{HTTP_TIMEOUT, Paths};
use autono_protocol::{API_VERSION, ApiError, DaemonMetadata, Event, EventKind, Run, Submission};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

pub struct Client {
    http: reqwest::Client,
    metadata: DaemonMetadata,
    paths: Paths,
}

impl Client {
    pub async fn connect(paths: &Paths) -> Result<Self> {
        let contents = tokio::fs::read(paths.metadata())
            .await
            .context("daemon metadata unavailable; start `autono daemon` in another terminal")?;
        let metadata: DaemonMetadata =
            serde_json::from_slice(&contents).context("parse daemon metadata")?;
        autono_daemon::validate_address(metadata.address)?;
        anyhow::ensure!(
            metadata.api_version == API_VERSION,
            "daemon API version mismatch"
        );
        anyhow::ensure!(metadata.token.len() >= 32, "invalid daemon token");
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(2))
            .build()?;
        Ok(Self {
            http,
            metadata,
            paths: paths.clone(),
        })
    }

    fn request(&self, method: reqwest::Method, route: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("http://{}{route}", self.metadata.address))
            .bearer_auth(&self.metadata.token)
    }

    pub async fn get<T: DeserializeOwned>(&self, route: &str) -> Result<T> {
        let response = self
            .request(reqwest::Method::GET, route)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .context("contact daemon; start `autono daemon` if it is stopped")?;
        checked(response)
            .await?
            .json()
            .await
            .context("decode daemon response")
    }

    pub async fn post<T: DeserializeOwned>(&self, route: &str, body: &impl Serialize) -> Result<T> {
        let response = self
            .request(reqwest::Method::POST, route)
            .timeout(HTTP_TIMEOUT)
            .json(body)
            .send()
            .await
            .context("contact daemon")?;
        checked(response)
            .await?
            .json()
            .await
            .context("decode daemon response")
    }

    pub async fn submit(&mut self, submission: &Submission) -> Result<Run> {
        for attempt in 0..3 {
            match self
                .request(reqwest::Method::POST, "/v1/runs")
                .timeout(HTTP_TIMEOUT)
                .json(submission)
                .send()
                .await
            {
                Ok(response) if response.status() != reqwest::StatusCode::UNAUTHORIZED => {
                    let response = checked(response).await?;
                    match response.json::<Run>().await {
                        Ok(run) => return Ok(run),
                        Err(error) if attempt == 2 => {
                            return Err(error).context(
                                "accepted response lost; retry with the same --submission-key",
                            );
                        }
                        Err(_) => {}
                    }
                }
                Ok(_) if attempt == 2 => anyhow::bail!("daemon authentication failed"),
                Err(error) if attempt == 2 => {
                    return Err(error).context(
                        "submission response unavailable; retry using the same --submission-key",
                    );
                }
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_millis(100 << attempt)).await;
            if let Ok(client) = Self::connect(&self.paths).await {
                self.metadata = client.metadata;
            }
        }
        anyhow::bail!("submission retry budget exhausted")
    }

    pub async fn follow(
        &mut self,
        id: Uuid,
        mut after: i64,
        mut emit: impl FnMut(Event) -> Result<()>,
    ) -> Result<()> {
        for attempt in 0..=5 {
            match self.follow_once(id, &mut after, &mut emit).await {
                Ok(()) => return Ok(()),
                Err(error) if error.is::<OutputFailed>() => return Err(error),
                Err(error) if attempt == 5 => {
                    return Err(error).with_context(|| {
                        format!("log reconnect budget exhausted; resume with --after {after}")
                    });
                }
                Err(error) => eprintln!(
                    "log stream disconnected ({error}); reconnecting after sequence {after}"
                ),
            }
            tokio::time::sleep(std::time::Duration::from_millis(100 << attempt)).await;
            if let Ok(client) = Self::connect(&self.paths).await {
                self.metadata = client.metadata;
            }
        }
        anyhow::bail!("log reconnect budget exhausted")
    }

    async fn follow_once(
        &self,
        id: Uuid,
        after: &mut i64,
        emit: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<()> {
        let response = tokio::time::timeout(
            HTTP_TIMEOUT,
            self.request(
                reqwest::Method::GET,
                &format!("/v1/runs/{id}/events/stream?after={after}"),
            )
            .send(),
        )
        .await??;
        let mut response = checked(response).await?;
        let mut buffer = Vec::new();
        loop {
            let chunk = tokio::time::timeout(std::time::Duration::from_secs(20), response.chunk())
                .await
                .context("event stream heartbeat timed out")??;
            let Some(bytes) = chunk else {
                break;
            };
            buffer.extend(bytes);
            while let Some(end) = buffer.windows(2).position(|w| w == b"\n\n") {
                let frame: Vec<u8> = buffer.drain(..end + 2).collect();
                let text = std::str::from_utf8(&frame).context("invalid SSE frame")?;
                if text.lines().any(|line| line == "event: error") {
                    anyhow::bail!("daemon event stream failed");
                }
                if let Some(data) = text.lines().find_map(|line| line.strip_prefix("data: ")) {
                    let event: Event = serde_json::from_str(data).context("decode run event")?;
                    if event.sequence <= *after {
                        continue;
                    }
                    let sequence = event.sequence;
                    let terminal = matches!(&event.kind, EventKind::StateChanged { to, .. } if to.is_terminal());
                    emit(event).map_err(OutputFailed)?;
                    *after = sequence;
                    if terminal {
                        return Ok(());
                    }
                }
            }
            anyhow::ensure!(buffer.len() <= 128 * 1024, "oversized event stream frame");
        }
        anyhow::ensure!(buffer.is_empty(), "incomplete event stream frame");
        // Observe terminal state before fetching the final tail: a graceful daemon
        // shutdown can close SSE just before the terminal transaction commits.
        let run: Run = self.get(&format!("/v1/runs/{id}")).await?;
        anyhow::ensure!(
            run.status.is_terminal(),
            "stream closed before terminal event"
        );
        loop {
            let pending: Vec<Event> = self
                .get(&format!("/v1/runs/{id}/events?after={after}"))
                .await?;
            if pending.is_empty() {
                break;
            }
            for event in pending {
                let sequence = event.sequence;
                emit(event).map_err(OutputFailed)?;
                *after = sequence;
            }
        }
        Ok(())
    }
}

async fn checked(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = response
        .json::<ApiError>()
        .await
        .map(|body| body.error)
        .unwrap_or_else(|_| "unstructured daemon error".into());
    anyhow::bail!("daemon returned {status}: {message}")
}

#[derive(Debug)]
struct OutputFailed(anyhow::Error);
impl std::fmt::Display for OutputFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "write log output: {}", self.0)
    }
}
impl std::error::Error for OutputFailed {}
