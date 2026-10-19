use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use autono_core::{
    Run, Supervisor,
    store::{InvalidTransition, RunNotFound},
    supervisor::NotActive,
};
use autono_protocol::{API_VERSION, ApiError, CancelResponse, HostStatus, RunView, Submission};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
struct App {
    supervisor: Supervisor,
    token: String,
    authority: String,
    stop: CancellationToken,
    body_timeout: Duration,
    submissions: Arc<tokio::sync::Semaphore>,
    streams: Arc<tokio::sync::Semaphore>,
}

pub fn router(
    supervisor: Supervisor,
    token: String,
    address: SocketAddr,
    stop: CancellationToken,
) -> Router {
    configured_router(supervisor, token, address, stop, Duration::from_secs(5))
}

pub fn configured_router(
    supervisor: Supervisor,
    token: String,
    address: SocketAddr,
    stop: CancellationToken,
    body_timeout: Duration,
) -> Router {
    let state = Arc::new(App {
        supervisor,
        token,
        authority: address.to_string(),
        stop,
        body_timeout,
        submissions: Arc::new(tokio::sync::Semaphore::new(32)),
        streams: Arc::new(tokio::sync::Semaphore::new(32)),
    });
    Router::new()
        .route("/v1/status", get(status))
        .route("/v1/runs", get(list).post(submit))
        .route("/v1/runs/{id}", get(inspect))
        .route("/v1/runs/{id}/cancel", post(cancel))
        .route("/v1/runs/{id}/events", get(events))
        .route("/v1/runs/{id}/events/stream", get(stream))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
}

async fn authenticate(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    // Browser origins are intentionally unsupported; no CORS or cookies. Exact Host also
    // prevents DNS rebinding. Token is never accepted in a query string or event URL.
    if request.headers().contains_key(header::ORIGIN)
        || request.headers().get_all(header::HOST).iter().count() != 1
        || request
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            != Some(&app.authority)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiError {
                code: "forbidden".into(),
                error: "browser origins and foreign hosts are forbidden".into(),
            }),
        )
            .into_response();
    }
    let expected = format!("Bearer {}", app.token);
    let authorized = request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        == 1
        && request
            .headers()
            .get(header::AUTHORIZATION)
            .is_some_and(|value| constant_time_equal(value.as_bytes(), expected.as_bytes()));
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                code: "unauthorized".into(),
                error: "bearer token required".into(),
            }),
        )
            .into_response();
    }
    // Keep complete/incomplete submissions and slow streams below the HTTP
    // connection bound so operational saturation leaves room for control routes.
    let _submission =
        if request.method() == axum::http::Method::POST && request.uri().path() == "/v1/runs" {
            match app.submissions.clone().try_acquire_owned() {
                Ok(slot) => Some(slot),
                Err(_) => {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(ApiError {
                            code: "overloaded".into(),
                            error: "HTTP submission capacity exhausted".into(),
                        }),
                    )
                        .into_response();
                }
            }
        } else {
            None
        };
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(app.body_timeout, to_bytes(body, 1024 * 1024)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(ApiError {
                    code: "invalid_body".into(),
                    error: "request body failed or exceeded 1 MiB".into(),
                }),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::REQUEST_TIMEOUT,
                Json(ApiError {
                    code: "body_timeout".into(),
                    error: "request body read timed out".into(),
                }),
            )
                .into_response();
        }
    };
    let mut response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    bool::from(left.ct_eq(right))
}

struct Failure(anyhow::Error);
impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, code) = if self.0.is::<RunNotFound>() {
            (StatusCode::NOT_FOUND, "not_found")
        } else if self.0.is::<autono_core::supervisor::Overloaded>() {
            (StatusCode::TOO_MANY_REQUESTS, "overloaded")
        } else if self.0.is::<autono_core::supervisor::ShuttingDown>() {
            (StatusCode::SERVICE_UNAVAILABLE, "shutting_down")
        } else if self.0.is::<autono_core::supervisor::PersistenceDegraded>() {
            (StatusCode::SERVICE_UNAVAILABLE, "persistence_degraded")
        } else if self.0.is::<autono_core::store::SubmissionConflict>() {
            (StatusCode::CONFLICT, "submission_conflict")
        } else if self.0.is::<NotActive>() || self.0.is::<InvalidTransition>() {
            (StatusCode::CONFLICT, "not_active")
        } else {
            tracing::error!(error = %format!("{:#}", self.0), "API request failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        };
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
            "internal runtime error; see daemon logs".into()
        } else {
            self.0.to_string()
        };
        (
            status,
            Json(ApiError {
                code: code.into(),
                error: message,
            }),
        )
            .into_response()
    }
}

async fn status(State(app): State<Arc<App>>) -> Result<Json<HostStatus>, Failure> {
    let database =
        match tokio::time::timeout(Duration::from_millis(200), app.supervisor.store().ping()).await
        {
            Ok(Ok(())) => "available",
            _ => "busy_or_unavailable",
        };
    Ok(Json(HostStatus {
        api_version: API_VERSION,
        pid: std::process::id(),
        platform: std::env::consts::OS.into(),
        database: database.into(),
        admission: app.supervisor.admission(),
        power: app.supervisor.power().status().await,
        persistence: app.supervisor.persistence(),
    }))
}

#[derive(Default, Deserialize)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
    after: Option<i64>,
}

async fn list(
    State(app): State<Arc<App>>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<RunView>>, Failure> {
    let runs = app
        .supervisor
        .store()
        .list(page.limit.unwrap_or(100), page.offset.unwrap_or(0))
        .await?;
    Ok(Json(
        runs.into_iter()
            .map(|run| run_view(&app.supervisor, run))
            .collect(),
    ))
}
async fn submit(State(app): State<Arc<App>>, Json(submission): Json<Submission>) -> Response {
    if let Err(error) = submission.spec.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "invalid_submission".into(),
                error: error.to_string(),
            }),
        )
            .into_response();
    }
    match app.supervisor.submit_keyed(submission).await {
        Ok(run) => (StatusCode::ACCEPTED, Json(run_view(&app.supervisor, run))).into_response(),
        Err(error) => Failure(error).into_response(),
    }
}
fn run_view(supervisor: &Supervisor, run: Run) -> RunView {
    RunView {
        runtime: supervisor.ownership(run.id),
        run,
    }
}
async fn inspect(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Json<RunView>, Failure> {
    Ok(Json(run_view(
        &app.supervisor,
        app.supervisor.store().get(id).await?,
    )))
}
async fn cancel(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Json<CancelResponse>, Failure> {
    app.supervisor.cancel(id).await?;
    Ok(Json(CancelResponse { accepted: true }))
}
async fn events(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<autono_protocol::Event>>, Failure> {
    app.supervisor.store().get(id).await?;
    Ok(Json(
        app.supervisor
            .store()
            .events(id, page.after.unwrap_or(0), page.limit.unwrap_or(500))
            .await?,
    ))
}

async fn stream(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
    headers: axum::http::HeaderMap,
) -> Result<Response, Failure> {
    let slot = app
        .streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| Failure(autono_core::supervisor::Overloaded("event streams").into()))?;
    app.supervisor.store().get(id).await?;
    let mut after = page.after.unwrap_or(0);
    if let Some(cursor) = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        after = after.max(cursor);
    }
    let output = async_stream::stream! {
        let _slot = slot;
        let mut changed = app.supervisor.store().subscribe();
        'stream: loop {
            let batch = match app.supervisor.store().events(id, after, 500).await {
                Ok(batch) => batch,
                Err(error) => {
                    tracing::error!(%error, "event stream database failure");
                    yield Ok::<_, Infallible>(Event::default().event("error").data("database read failed; reconnect with last-event-id"));
                    break;
                }
            };
            if !batch.is_empty() {
                for event in batch {
                    let sequence = event.sequence;
                    match Event::default().event("run_event").id(sequence.to_string()).json_data(&event) {
                        Ok(event) => { after = sequence; yield Ok(event); },
                        Err(error) => {
                            tracing::error!(%error, "serialize stream event");
                            yield Ok(Event::default().event("error").data("event serialization failed"));
                            break 'stream;
                        }
                    }
                }
                continue;
            }
            // State and terminal event commit together. Check again after observing terminal
            // state so an event committed between the previous reads cannot be skipped.
            match app.supervisor.store().get(id).await {
                Ok(run) if run.status.is_terminal() => {
                    match app.supervisor.store().events(id, after, 1).await {
                        Ok(events) if !events.is_empty() => continue,
                        Ok(_) => break,
                        Err(error) => {
                            tracing::error!(%error, "read terminal stream events");
                            yield Ok(Event::default().event("error").data("database read failed; reconnect with last-event-id"));
                            break;
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "inspect streaming run");
                    yield Ok(Event::default().event("error").data("database read failed; reconnect with last-event-id"));
                    break;
                }
                _ => {},
            }
            tokio::select! {
                _ = app.stop.cancelled() => break,
                result = changed.changed() => { if result.is_err() { break; } },
            }
        }
    };
    Ok(Sse::new(output)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response())
}
