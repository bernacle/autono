//! Fixed-route desktop client. This module never starts a process or owns daemon lifetime.
use autono_core::config::{Config, Paths};
use autono_protocol::engineering::{EngineeringRun, PlanSubmission, ResolutionRequest};
use autono_protocol::{API_VERSION, ApiError, DaemonMetadata, Event, EventKind, RunView};
use base64::{Engine, engine::general_purpose::STANDARD};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, io::Read, path::PathBuf, sync::OnceLock, time::Duration};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}
impl BridgeError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
type Result<T> = std::result::Result<T, BridgeError>;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    Runs {
        offset: u32,
    },
    Run {
        id: Uuid,
    },
    Events {
        id: Uuid,
        after: i64,
    },
    Processes {
        offset: u32,
    },
    ProcessEvents {
        id: Uuid,
        after: i64,
    },
    Settings,
    Plan {
        submission: PlanSubmission,
    },
    Action {
        id: Uuid,
        revision: i64,
        action: Action,
    },
    Resolve {
        id: Uuid,
        revision: i64,
        action_id: String,
        resolution: ResolutionRequest,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Execute,
    Preflight,
    Continue,
    Cancel,
    Integrate,
}
impl Action {
    fn route(&self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Preflight => "preflight",
            Self::Continue => "continue",
            Self::Cancel => "cancel",
            Self::Integrate => "integrate",
        }
    }
}

pub struct Bridge {
    paths: Paths,
    metadata: DaemonMetadata,
    http: reqwest::Client,
    redactor: Redactor,
}
impl Bridge {
    pub fn discover() -> Result<Self> {
        Self::connect(
            Paths::discover(None)
                .map_err(|_| BridgeError::new("metadata", "Cannot locate Autono home"))?,
        )
    }
    pub fn connect(paths: Paths) -> Result<Self> {
        let metadata = read_metadata(&paths)?;
        let redactor = Redactor::new(&metadata.token);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .build()
            .map_err(|_| {
                BridgeError::new("connection", "Could not initialize local HTTP client")
            })?;
        Ok(Self {
            paths,
            metadata,
            http,
            redactor,
        })
    }
    async fn request(&self, route: &str, body: Option<Value>) -> Result<Value> {
        let method = if body.is_some() {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };
        let mut request = self
            .http
            .request(method, format!("http://{}{route}", self.metadata.address))
            .bearer_auth(&self.metadata.token)
            .timeout(Duration::from_secs(if body.is_some() { 120 } else { 10 }));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| BridgeError::new("disconnected", "Daemon unavailable or restarting. Reconnect to reread local connection metadata; inspect run state before retrying an action."))?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            BridgeError::new(
                "disconnected",
                "Daemon response interrupted; inspect current state before retrying",
            )
        })? {
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err(BridgeError::new(
                    "response_limit",
                    "Response exceeded 16 MiB; use the CLI for this large aggregate",
                ));
            }
            bytes.extend(chunk);
        }
        if !status.is_success() {
            let error = serde_json::from_slice::<ApiError>(&bytes).ok();
            return Err(BridgeError::new(
                if status.as_u16() == 401 {
                    "authentication"
                } else {
                    "rejected"
                },
                error
                    .map(|e| self.redactor.text(&e.error))
                    .unwrap_or_else(|| format!("Daemon rejected request ({status})")),
            ));
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            BridgeError::new(
                "protocol",
                "Invalid daemon response; check compatible API versions",
            )
        })
    }
    async fn current(&self, id: Uuid, revision: i64) -> Result<()> {
        let run = self.request(&format!("/v1/engineering/{id}"), None).await?;
        if run["revision"].as_i64() != Some(revision) {
            return Err(BridgeError::new(
                "stale",
                "Run changed since inspection. Refresh and review the current state before confirming again.",
            ));
        }
        Ok(())
    }
    pub async fn dispatch(&self, request: Request) -> Result<Value> {
        let mut result = match request {
            Request::Status => {
                let (host, engineering) = tokio::try_join!(
                    self.request("/v1/status", None),
                    self.request("/v1/engineering/health", None)
                )?;
                if host["api_version"].as_u64() != Some(API_VERSION as u64)
                    || host["pid"].as_u64() != Some(self.metadata.pid as u64)
                {
                    return Err(BridgeError::new(
                        "metadata",
                        "Connection metadata is stale or incompatible. Reconnect after the daemon restarts.",
                    ));
                }
                json!({"host":host,"engineering":engineering,"address":self.metadata.address.to_string(),"home":self.paths.home})
            }
            Request::Runs { offset } => {
                self.request(&format!("/v1/engineering?limit=20&offset={offset}"), None)
                    .await?
            }
            Request::Run { id } => self.request(&format!("/v1/engineering/{id}"), None).await?,
            Request::Events { id, after } => {
                if after < 0 {
                    return Err(BridgeError::new("cursor", "Cursor must be nonnegative"));
                }
                self.request(
                    &format!("/v1/engineering/{id}/events?limit=100&after={after}"),
                    None,
                )
                .await?
            }
            Request::Processes { offset } => {
                let value = self
                    .request(&format!("/v1/runs?limit=50&offset={offset}"), None)
                    .await?;
                // Explicit projection: command argv, provider payloads and process env never leave Rust.
                let runs: Vec<RunView> = serde_json::from_value(value)
                    .map_err(|_| BridgeError::new("protocol", "Invalid process list"))?;
                Value::Array(runs.into_iter().map(|r| json!({"id":r.run.id,"status":r.run.status,"pid":r.run.pid,"cwd":r.run.spec.cwd,"program":r.run.spec.command.first().and_then(|p| std::path::Path::new(p).file_name()).map(|p|p.to_string_lossy()),"created_at":r.run.created_at,"detail":r.run.detail,"runtime":r.runtime,"output_complete":r.run.output_complete})).collect())
            }
            Request::ProcessEvents { id, after } => return self.process_events(id, after).await,
            Request::Settings => {
                let config = Config::load(&self.paths).await.map_err(|_| {
                    BridgeError::new(
                        "configuration",
                        "User configuration cannot be parsed; inspect config.toml locally",
                    )
                })?;
                let available: Vec<_> = ["git", "codex", "claude"]
                    .into_iter()
                    .map(|name| {
                        let found = std::env::var_os("PATH").is_some_and(|path| {
                            std::env::split_paths(&path).any(|dir| {
                                let p = dir.join(name);
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    p.metadata().is_ok_and(|m| {
                                        m.is_file() && m.permissions().mode() & 0o111 != 0
                                    })
                                }
                                #[cfg(not(unix))]
                                {
                                    p.is_file()
                                }
                            })
                        });
                        json!({"name":name,"available":found})
                    })
                    .collect();
                json!({"home":self.paths.home,"config_path":self.paths.config(),"source":"User file on disk; daemon may need restart. Project overrides are snapshotted per run.","agents":config.agents,"execution":config.execution,"verification":config.verification,"runner":config.runner,"cli":available})
            }
            Request::Plan { submission } => {
                self.request(
                    "/v1/engineering",
                    Some(
                        serde_json::to_value(submission)
                            .map_err(|_| BridgeError::new("input", "Invalid plan"))?,
                    ),
                )
                .await?
            }
            Request::Action {
                id,
                revision,
                action,
            } => {
                self.current(id, revision).await?;
                self.request(
                    &format!("/v1/engineering/{id}/{}", action.route()),
                    Some(json!({})),
                )
                .await?
            }
            Request::Resolve {
                id,
                revision,
                action_id,
                resolution,
            } => {
                if action_id.is_empty()
                    || action_id.len() > 128
                    || !action_id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                {
                    return Err(BridgeError::new("input", "Invalid human action identifier"));
                }
                // A lost response can be retried with its original receipt even after revision changes.
                let run = self.request(&format!("/v1/engineering/{id}"), None).await?;
                let prior: EngineeringRun = serde_json::from_value(run)
                    .map_err(|_| BridgeError::new("protocol", "Invalid engineering run"))?;
                let receipt = prior
                    .human_actions
                    .iter()
                    .find(|a| a.id == action_id)
                    .is_some_and(|a| {
                        a.audit
                            .iter()
                            .any(|a| a.request.as_ref() == Some(&resolution))
                    });
                if !receipt && prior.revision != revision {
                    return Err(BridgeError::new(
                        "stale",
                        "Run changed; inspect current action before resolving",
                    ));
                }
                self.request(
                    &format!("/v1/engineering/{id}/blockers/{action_id}/resolve"),
                    Some(
                        serde_json::to_value(resolution)
                            .map_err(|_| BridgeError::new("input", "Invalid resolution"))?,
                    ),
                )
                .await?
            }
        };
        self.redactor.value(&mut result);
        Ok(result)
    }
    pub async fn inspect_artifact(&self, run_id: Uuid, path: String) -> Result<Value> {
        let run: EngineeringRun = serde_json::from_value(
            self.request(&format!("/v1/engineering/{run_id}"), None)
                .await?,
        )
        .map_err(|_| BridgeError::new("protocol", "Invalid run"))?;
        let mut allowed = vec![run.planning_workspace.path];
        allowed.extend(run.integration_workspace.into_iter().map(|w| w.path));
        allowed.extend(run.tasks.into_iter().flat_map(|t| {
            t.workspace
                .into_iter()
                .map(|w| w.path)
                .chain(t.attempts.into_iter().flat_map(|a| {
                    a.reviews
                        .into_iter()
                        .filter_map(|r| r.workspace.map(|w| w.path))
                }))
        }));
        let requested = PathBuf::from(path);
        if !allowed.contains(&requested) {
            return Err(BridgeError::new(
                "artifact",
                "Only daemon-associated workspace directories can be inspected",
            ));
        }
        accessible_directory(requested).await
    }

    async fn process_events(&self, id: Uuid, after: i64) -> Result<Value> {
        static READERS: OnceLock<Mutex<HashMap<Uuid, LogReader>>> = OnceLock::new();
        let mut readers = READERS.get_or_init(Default::default).lock().await;
        if !readers.contains_key(&id) && readers.len() >= 32 {
            readers.clear();
        }
        let reader = readers.entry(id).or_default();
        if let Some((cursor, cached)) = &reader.last
            && *cursor == after
        {
            return Ok(cached.clone());
        }
        if after == 0 {
            *reader = LogReader::default();
            let process: RunView =
                serde_json::from_value(self.request(&format!("/v1/runs/{id}"), None).await?)
                    .map_err(|_| BridgeError::new("protocol", "Invalid process record"))?;
            reader.secrets = process
                .run
                .spec
                .env
                .into_iter()
                .filter(|(key, value)| sensitive(key) && !value.is_empty())
                .map(|(_, value)| value)
                .collect();
        }
        let mut redactor = Redactor {
            literals: self.redactor.literals.clone(),
        };
        redactor.literals.extend(reader.secrets.clone());
        redactor
            .literals
            .sort_by_key(|v| std::cmp::Reverse(v.len()));
        if reader.cursor != after {
            return Err(BridgeError::new(
                "cursor_reset",
                "Log reader expired. Replay from the beginning.",
            ));
        }
        let events: Vec<Event> = serde_json::from_value(
            self.request(
                &format!("/v1/runs/{id}/events?after={after}&limit=100"),
                None,
            )
            .await?,
        )
        .map_err(|_| BridgeError::new("protocol", "Invalid process events"))?;
        let mut lines = Vec::new();
        for event in &events {
            match &event.kind {
                EventKind::Output { stream, data } => {
                    let bytes = STANDARD.decode(data).map_err(|_| BridgeError::new("protocol", "Invalid output encoding"))?;
                    let index = match stream { autono_protocol::OutputStream::Stdout => 0, autono_protocol::OutputStream::Stderr => 1 };
                    for byte in bytes { if byte == b'\n' {
                        if !reader.discard[index] { lines.push(json!({"sequence":event.sequence,"timestamp":event.timestamp,"stream":stream,"text":redactor.output_line(&String::from_utf8_lossy(&reader.pending[index]))})); }
                        reader.pending[index].clear(); reader.discard[index] = false;
                    } else if reader.pending[index].len() < 16 * 1024 && !reader.discard[index] { reader.pending[index].push(byte); }
                    else { reader.pending[index].clear(); reader.discard[index] = true; }
                    }
                }
                EventKind::Warning { message } => lines.push(json!({"sequence":event.sequence,"timestamp":event.timestamp,"stream":"warning","text":redactor.text(message)})),
                EventKind::StateChanged { to, detail, .. } => {
                    if to.is_terminal() { for index in 0..2 { if !reader.pending[index].is_empty() && !reader.discard[index] { lines.push(json!({"sequence":event.sequence,"timestamp":event.timestamp,"stream":if index==0 {"stdout"}else{"stderr"},"text":redactor.output_line(&String::from_utf8_lossy(&reader.pending[index]))})); } reader.pending[index].clear(); } }
                    lines.push(json!({"sequence":event.sequence,"timestamp":event.timestamp,"stream":"state","text":format!("{}: {}",to.as_str(),redactor.text(detail.as_deref().unwrap_or("")))}));
                }
                _ => {}
            }
            reader.cursor = event.sequence;
        }
        let page = json!({"after":reader.cursor,"lines":lines,"has_more":events.len()==100});
        // Cache only a page that advanced; an empty poll must observe newly appended events.
        reader.last = if reader.cursor > after {
            Some((after, page.clone()))
        } else {
            None
        };
        Ok(page)
    }
}
async fn accessible_directory(requested: PathBuf) -> Result<Value> {
    let metadata = tokio::fs::symlink_metadata(&requested)
        .await
        .map_err(|_| BridgeError::new("artifact", "Workspace is missing or inaccessible"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(BridgeError::new(
            "artifact",
            "Workspace is not a regular directory",
        ));
    }
    // Opening the directory verifies access; stat alone succeeds on an unreadable directory.
    let _directory = tokio::fs::read_dir(&requested)
        .await
        .map_err(|_| BridgeError::new("artifact", "Workspace directory cannot be opened"))?;
    Ok(json!({"path":requested,"accessible":true}))
}

#[derive(Default)]
struct LogReader {
    cursor: i64,
    pending: [Vec<u8>; 2],
    secrets: Vec<String>,
    discard: [bool; 2],
    last: Option<(i64, Value)>,
}

fn read_metadata(paths: &Paths) -> Result<DaemonMetadata> {
    let read = || -> anyhow::Result<DaemonMetadata> {
        let home = std::fs::symlink_metadata(&paths.home)?;
        anyhow::ensure!(
            home.is_dir() && !home.file_type().is_symlink(),
            "invalid home"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            // SAFETY: geteuid has no preconditions.
            anyhow::ensure!(
                home.uid() == unsafe { libc::geteuid() } && home.permissions().mode() & 0o077 == 0,
                "insecure home"
            );
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(paths.metadata())?;
        let info = file.metadata()?;
        anyhow::ensure!(
            info.is_file() && info.len() <= 16 * 1024,
            "invalid metadata"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            // SAFETY: geteuid has no preconditions.
            anyhow::ensure!(
                info.uid() == unsafe { libc::geteuid() } && info.permissions().mode() & 0o077 == 0,
                "insecure metadata"
            );
        }
        let mut bytes = Vec::new();
        file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
        let metadata: DaemonMetadata = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            metadata.api_version == API_VERSION
                && metadata.token.len() >= 32
                && metadata.token.len() <= 512
                && metadata.address.ip().is_loopback()
                && metadata.address.port() != 0,
            "invalid connection"
        );
        Ok(metadata)
    };
    read().map_err(|_| BridgeError::new("metadata", "Local connection metadata is missing, expired, incompatible or insecure. Start the existing daemon separately; check AUTONO_HOME and owner-only permissions."))
}

struct Redactor {
    literals: Vec<String>,
}
impl Redactor {
    fn new(token: &str) -> Self {
        let mut literals = vec![token.to_owned()];
        literals.extend(
            std::env::vars_os()
                .filter(|(key, value)| sensitive(&key.to_string_lossy()) && value.len() >= 6)
                .filter_map(|(_, value)| value.into_string().ok()),
        );
        literals.sort_by_key(|v| std::cmp::Reverse(v.len()));
        Self { literals }
    }
    fn text(&self, text: &str) -> String {
        static PATTERN: OnceLock<Regex> = OnceLock::new();
        let marker = self.placeholder();
        let mut text = text.to_owned();
        for secret in &self.literals {
            text = text.replace(secret, &marker);
        }
        text = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect();
        // Control normalization can join fragments into a known credential.
        for secret in &self.literals {
            text = text.replace(secret, &marker);
        }
        let regex = PATTERN.get_or_init(|| Regex::new(r#"(?i)((?:bearer|basic)\s+[^\s"'<>]+|(?:[\w-]*(?:token|password|secret|api[_-]?key|credential)[\w-]*)["']?\s*[:=]\s*["']?[^\s,"'<>}]+|sk-[a-zA-Z0-9_-]{12,}|(?:https?://)[^\s/@:]+:[^\s/@]+@[^\s]+)"#).expect("valid redaction pattern"));
        let text = regex.replace_all(&text, marker.as_str()).into_owned();
        // Fail closed if replacements themselves reconstruct a known literal.
        if self.contains_literal(&text) {
            marker
        } else {
            text
        }
    }
    fn contains_literal(&self, text: &str) -> bool {
        self.literals
            .iter()
            .any(|secret| !secret.is_empty() && text.contains(secret))
    }
    fn placeholder(&self) -> String {
        for candidate in ["[redacted]", "[withheld]", "***", "…"] {
            if !self.contains_literal(candidate) {
                return candidate.into();
            }
        }
        // Even short process credentials must not occur in the replacement itself.
        // Prefer a nonempty marker so redacting a blocker cannot make it empty.
        (0xe000..=0x10ffff)
            .filter_map(char::from_u32)
            .filter(|c| !c.is_control())
            .map(|c| c.to_string())
            .find(|s| !self.contains_literal(s))
            .unwrap_or_default()
    }
    fn output_line(&self, text: &str) -> String {
        if let Ok(mut value) = serde_json::from_str::<Value>(text) {
            self.value(&mut value);
            let serialized = value.to_string();
            // JSON escaping must not reconstruct a known literal on the wire.
            return if self.contains_literal(&serialized) {
                self.placeholder()
            } else {
                serialized
            };
        }
        self.text(text)
    }
    fn value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.text(text),
            Value::Array(values) => {
                let mut redact_next = false;
                for v in values {
                    let flag = v
                        .as_str()
                        .is_some_and(|s| s.starts_with('-') && sensitive(s) && !s.contains('='));
                    if redact_next {
                        *v = Value::String(self.placeholder());
                    } else {
                        self.value(v);
                    }
                    redact_next = flag;
                }
            }
            Value::Object(map) => {
                if map.get("type").and_then(Value::as_str) == Some("item.completed")
                    && let Some(item) = map.get_mut("item").and_then(Value::as_object_mut)
                {
                    item.remove("text");
                }
                let original = std::mem::take(map);
                for (key, mut v) in original {
                    // ReviewResult::accepts uses is_empty(), not trim(): retain
                    // rejection when a nonempty blocker consists only of controls.
                    let had_blocker = key == "blocker" && v.as_str().is_some_and(|s| !s.is_empty());
                    if sensitive(&key)
                        || ["env", "result", "structured_output"].contains(&key.as_str())
                    {
                        v = Value::String(self.placeholder());
                    } else {
                        self.value(&mut v);
                    }
                    if had_blocker && v.as_str() == Some("") {
                        v = Value::String(self.placeholder());
                    }
                    let key = self.text(&key);
                    match map.entry(key) {
                        serde_json::map::Entry::Vacant(entry) => {
                            entry.insert(v);
                        }
                        // Redacted/normalized keys can collide. Never keep either payload.
                        serde_json::map::Entry::Occupied(mut entry) => {
                            entry.insert(Value::String(self.placeholder()));
                        }
                    }
                }
            }
            _ => {
                if self.contains_literal(&value.to_string()) {
                    *value = Value::String(self.placeholder());
                }
            }
        }
    }
}
fn sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    [
        "token",
        "password",
        "secret",
        "api_key",
        "apikey",
        "credential",
        "authorization",
        "cookie",
        "private_key",
    ]
    .iter()
    .any(|s| key.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_known_secret_is_redacted_even_in_json_object_keys() {
        let secret = "pr4-known-credential-canary-1234567890";
        let r = Redactor::new(secret);
        let line = json!({(secret): "ordinary value"}).to_string();
        assert!(!r.output_line(&line).contains(secret));
    }
    fn assert_no_literal(value: &Value, literals: &[&str]) {
        match value {
            Value::String(text) => assert!(literals.iter().all(|s| !text.contains(s))),
            Value::Array(values) => values.iter().for_each(|v| assert_no_literal(v, literals)),
            Value::Object(map) => {
                for (key, value) in map {
                    assert!(literals.iter().all(|s| !key.contains(s)));
                    assert_no_literal(value, literals);
                }
            }
            _ => assert!(literals.iter().all(|s| !value.to_string().contains(s))),
        }
    }
    #[test]
    fn redacts_decoded_keys_nested_arrays_normalization_and_collisions() {
        let secret = "canary-1234567890";
        let r = Redactor {
            literals: vec![secret.into(), "123456".into()],
        };
        let interrupted = "canary-\u{1b}1234567890".to_owned();
        let payload = json!({
            (secret): [{"nested":{(secret):secret}}, 123456],
            (interrupted): "normalized key payload",
            "[redacted]": "collision payload",
            "safe": "retained",
            "events":[{"type":"item.completed","item":{"text":"raw provider transcript"}}]
        });
        let mut direct = payload.clone();
        r.value(&mut direct);
        assert_no_literal(&direct, &[secret, "123456"]);
        assert_eq!(direct["[redacted]"], "[redacted]");
        assert_eq!(direct["safe"], "retained");
        assert!(!direct.to_string().contains("collision payload"));
        assert!(!direct.to_string().contains("raw provider transcript"));
        for value in [payload.clone(), json!([payload]), json!(secret)] {
            let escaped = value.to_string().replace("canary", "\\u0063anary");
            let output = r.output_line(&escaped);
            let decoded: Value = serde_json::from_str(&output).expect("sanitized valid JSON");
            assert_no_literal(&decoded, &[secret, "123456"]);
            assert!(!output.contains(secret));
        }
    }
    #[test]
    fn replacement_and_json_serialization_cannot_reintroduce_literals() {
        let r = Redactor {
            literals: vec!["redacted".into()],
        };
        let mut value =
            json!({"password":"private", "array":["--token","private"], "redacted": "value"});
        r.value(&mut value);
        assert_no_literal(&value, &["redacted"]);
        let mut blocker = json!({"blocker":"redacted"});
        r.value(&mut blocker);
        assert!(
            !blocker["blocker"]
                .as_str()
                .expect("retained blocker")
                .is_empty()
        );
        let short = Redactor {
            literals: vec!["redacted".into(), "withheld".into(), "*".into(), "…".into()],
        };
        let marker = short.placeholder();
        assert!(!marker.is_empty());
        assert!(!short.contains_literal(&marker));
        // The decoded value does not contain a backslash-n, but serialization does.
        let r = Redactor {
            literals: vec!["canary\\n".into()],
        };
        assert_eq!(r.output_line(&json!("canary\n").to_string()), "[redacted]");
    }
    #[test]
    fn blocker_projection_preserves_daemon_verdict_without_changing_general_text() {
        use autono_protocol::engineering::ReviewResult;
        let r = Redactor {
            literals: vec!["desktop-only-authentication-token-1234567890".into()],
        };
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../src/test/fixtures/review-blocker-cases.json"
        ))
        .expect("shared blocker cases");
        for case in cases {
            let original: ReviewResult = serde_json::from_value(json!({
                "accepted":true,"criteria":[{"criterion_id":"A-C1","criterion":"Safe criterion","satisfied":true,"evidence":"Readable evidence"}],
                "findings":[],"blocker":case["original"]
            })).expect("valid review");
            let mut value = json!({"review":original, "reviews":[{"review":original}], "ordinary_text":"\u{1b}"});
            r.value(&mut value);
            assert_eq!(value["ordinary_text"], "");
            for projected in [&value["review"], &value["reviews"][0]["review"]] {
                let projected: ReviewResult =
                    serde_json::from_value(projected.clone()).expect("projected review");
                assert_eq!(projected.accepts(), original.accepts(), "{}", case["name"]);
                assert_eq!(
                    projected.blocker,
                    case["projected"].as_str().expect("projected blocker")
                );
                assert_eq!(projected.criteria[0].evidence, "Readable evidence");
            }
        }
    }
    #[test]
    fn redacts_secrets_and_controls() {
        let r = Redactor::new("exact-daemon-bearer-abcdefghijklmnopqrstuvwxyz");
        let out = r.text("Bearer abcdef\u{1b}[31m API_KEY=abc123 password: hunter2 sk-abcdefghijklmnop exact-daemon-bearer-abcdefghijklmnopqrstuvwxyz");
        assert!(!out.contains("abcdef"));
        assert!(!out.contains("hunter2"));
        assert!(!out.contains('\u{1b}'));
        let mut value = json!({"env":{"PRIVATE":"value"},"token":"anything","result":"provider transcript","status":"RUNNING"});
        r.value(&mut value);
        assert_eq!(value["env"], "[redacted]");
        assert_eq!(value["status"], "RUNNING");
        let mut command = json!([
            "tool",
            "--password",
            "argument-canary",
            "--api-key",
            "key-canary",
            "safe"
        ]);
        r.value(&mut command);
        assert_eq!(
            command,
            json!([
                "tool",
                "--password",
                "[redacted]",
                "--api-key",
                "[redacted]",
                "safe"
            ])
        );
        assert!(!r.text("Authorization: Basic aGVsbG8=").contains("aGVsbG8="));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn directory_access_checks_open_permissions_and_rejects_missing_paths()
    -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir()?;
        let blocked = dir.path().join("blocked");
        std::fs::create_dir(&blocked)?;
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000))?;
        let result = accessible_directory(blocked.clone()).await;
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700))?;
        // SAFETY: geteuid has no preconditions. Root can open mode-000 directories.
        if unsafe { libc::geteuid() } != 0 {
            assert!(result.is_err());
        }
        assert!(
            accessible_directory(dir.path().join("missing"))
                .await
                .is_err()
        );
        assert!(accessible_directory(blocked).await.is_ok());
        Ok(())
    }
    #[test]
    fn rejects_missing_foreign_and_insecure_metadata() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths {
            home: dir.path().to_owned(),
        };
        assert!(Bridge::connect(paths.clone()).is_err());
        let metadata = json!({"api_version":1,"pid":1,"address":"192.0.2.1:1234","token":"abcdefghijklmnopqrstuvwxyz1234567890"});
        std::fs::write(paths.metadata(), metadata.to_string())?;
        assert!(Bridge::connect(paths).is_err());
        Ok(())
    }
}
