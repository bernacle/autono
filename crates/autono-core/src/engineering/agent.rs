//! CLI adapters use existing provider login. Authentication files are never accessed.
use super::{
    AgentConfig, AgentRole, Provider,
    process::{CommandOutput, ProcessRunner},
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{future::Future, path::PathBuf, pin::Pin};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub type AgentFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>>;
pub struct AgentRequest {
    pub bounded: bool,
    pub config: AgentConfig,
    pub role: AgentRole,
    pub cwd: PathBuf,
    pub prompt: String,
    pub schema_path: PathBuf,
    pub resume: Option<String>,
}
#[derive(Debug)]
pub struct AgentResponse {
    pub session_id: Option<String>,
    pub result: Value,
}
#[derive(Debug, thiserror::Error)]
#[error("{kind}: {detail}")]
pub struct AgentFailure {
    pub kind: String,
    pub detail: String,
    pub blocked: bool,
}
fn failure(kind: &str, detail: impl Into<String>, blocked: bool) -> anyhow::Error {
    AgentFailure {
        kind: kind.into(),
        detail: detail.into(),
        blocked,
    }
    .into()
}

/// Providers prepare noninteractive invocations and decode structured event streams.
/// The caller streams/persists output and owns cancellation/timeout through ProcessRunner.
pub type ReadinessFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<super::ReadinessCheck>>> + Send + 'a>>;
pub trait AgentRuntime: Send + Sync {
    /// Injected runtimes explicitly own their availability contract; no real CLI calls in CI.
    fn readiness<'a>(
        &'a self,
        _: &'a ProcessRunner,
        _: &'a std::path::Path,
        _: &'a super::WorkflowSettings,
        _: &'a CancellationToken,
    ) -> ReadinessFuture<'a> {
        Box::pin(async {
            Ok(vec![super::ReadinessCheck {
                id: "injected-provider".into(),
                ready: Some(true),
                blocking: true,
                evidence: "Injected runtime availability contract".into(),
            }])
        })
    }

    fn prepare<'a>(
        &'a self,
        request: &'a AgentRequest,
        runner: &'a ProcessRunner,
        stop: &'a CancellationToken,
    ) -> AgentFuture<'a>;
    fn decode(&self, provider: Provider, output: &CommandOutput) -> Result<AgentResponse>;
}
#[derive(Default)]
pub struct CliAgentRuntime;
impl AgentRuntime for CliAgentRuntime {
    fn readiness<'a>(
        &'a self,
        runner: &'a ProcessRunner,
        cwd: &'a std::path::Path,
        settings: &'a super::WorkflowSettings,
        stop: &'a CancellationToken,
    ) -> ReadinessFuture<'a> {
        Box::pin(async move {
            let mut checks = vec![];
            for (name, config) in [
                ("coder", &settings.agents.coder),
                ("reviewer", &settings.agents.reviewer),
            ] {
                let ready = self.probe(config.provider, runner, cwd, stop).await.is_ok();
                checks.push(super::ReadinessCheck { id: format!("provider-{name}"), ready: Some(ready), blocking: true, evidence: "CLI availability and noninteractive sandbox capability probes (15-second bounds); inspect installation if unavailable".into() });
                checks.push(super::ReadinessCheck { id: format!("authentication-{name}"), ready: None, blocking: false, evidence: "Unknown: credential files and credential-bearing authentication output are not inspected; use existing CLI login".into() });
            }
            Ok(checks)
        })
    }

    fn prepare<'a>(
        &'a self,
        request: &'a AgentRequest,
        runner: &'a ProcessRunner,
        stop: &'a CancellationToken,
    ) -> AgentFuture<'a> {
        Box::pin(async move {
            if request.bounded && request.config.provider != Provider::Claude {
                return Err(super::delegation::denied(
                    "provider cannot enforce tool-free bounded mode",
                ));
            }
            self.probe(request.config.provider, runner, &request.cwd, stop)
                .await?;
            if request.bounded {
                let help = runner
                    .run(
                        vec!["claude".into(), "--help".into()],
                        &request.cwd,
                        15,
                        Uuid::new_v4(),
                        stop,
                    )
                    .await?;
                require_probe_outcome(&help)?;
                let text = help.stdout_text()?;
                if !help.succeeded()
                    || [
                        "--restricted",
                        "--bare",
                        "--disallowedTools",
                        "--disable-slash-commands",
                        "--settings",
                        "--setting-sources",
                    ]
                    .iter()
                    .any(|flag| !text.contains(flag))
                {
                    return Err(super::delegation::denied(
                        "Claude lacks required restricted/tool-free settings controls",
                    ));
                }
            }
            let program = match request.config.provider {
                Provider::Codex => "codex",
                Provider::Claude => "claude",
            };
            let mut args: Vec<String>;
            match request.config.provider {
                Provider::Codex => {
                    let sandbox = if request.role == AgentRole::Coder {
                        "workspace-write"
                    } else {
                        "read-only"
                    };
                    args = vec![
                        program.into(),
                        "--no-daemon".into(),
                        "-a".into(),
                        "never".into(),
                        "-s".into(),
                        sandbox.into(),
                        "exec".into(),
                    ];
                    if let Some(session) = &request.resume {
                        let resume_help = runner
                            .run(
                                vec![
                                    program.into(),
                                    "exec".into(),
                                    "resume".into(),
                                    "--help".into(),
                                ],
                                &request.cwd,
                                15,
                                Uuid::new_v4(),
                                stop,
                            )
                            .await?;
                        require_probe_outcome(&resume_help)?;
                        if !resume_help.succeeded()
                            || !resume_help.stdout_text()?.contains("--output-schema")
                        {
                            return Err(failure(
                                "unsupported_resume",
                                "Codex resume lacks structured output support",
                                true,
                            ));
                        }
                        args.push("resume".into());
                        args.push(session.clone());
                    }
                    args.extend([
                        "--json".into(),
                        "--ignore-user-config".into(),
                        "--output-schema".into(),
                        request.schema_path.to_string_lossy().into_owned(),
                    ]);
                }
                Provider::Claude => {
                    // Safe mode keeps login while disabling hooks, plugins and project customizations.
                    // Restrict the exposed tool set; no Bash/MCP/edit tools in reviewer/planner sessions.
                    args = vec![
                        program.into(),
                        "--print".into(),
                        "--output-format".into(),
                        "stream-json".into(),
                        "--verbose".into(),
                        if request.bounded {
                            "--bare".into()
                        } else {
                            "--safe-mode".into()
                        },
                        "--strict-mcp-config".into(),
                        "--permission-prompts".into(),
                        "none".into(),
                        "--permission-mode".into(),
                        if !request.bounded && request.role == AgentRole::Coder {
                            "acceptEdits".into()
                        } else {
                            "dontAsk".into()
                        },
                        "--tools".into(),
                        if request.bounded {
                            "".into()
                        } else if request.role == AgentRole::Coder {
                            "Read,Glob,Grep,Edit,Write".into()
                        } else {
                            "Read,Glob,Grep".into()
                        },
                        "--json-schema".into(),
                        tokio::fs::read_to_string(&request.schema_path).await?,
                    ];
                    if request.bounded {
                        args.extend([
                            "--restricted".into(),
                            "--setting-sources".into(),
                            "".into(),
                            "--settings".into(),
                            "{\"disableAllHooks\":true}".into(),
                            "--disable-slash-commands".into(),
                            "--disallowedTools".into(),
                            "mcp__*".into(),
                        ]);
                    }
                    if !request.bounded
                        && let Some(parent) = request.schema_path.parent()
                    {
                        args.extend(["--add-dir".into(), parent.to_string_lossy().into_owned()]);
                    }
                    if let Some(session) = &request.resume {
                        args.extend(["--resume".into(), session.clone()]);
                    }
                }
            }
            if let Some(model) = &request.config.model {
                args.extend(["--model".into(), model.clone()]);
            }
            // Terminate variadic --tools/--add-dir parsing before the positional prompt.
            args.push("--".into());
            args.push(request.prompt.clone());
            Ok(args)
        })
    }
    fn decode(&self, provider: Provider, output: &CommandOutput) -> Result<AgentResponse> {
        decode(provider, output)
    }
}

impl CliAgentRuntime {
    /// Help-only, supervised capability detection. No inference, prompts, schema
    /// files, or invocation artifacts are required or constructed here.
    async fn probe(
        &self,
        provider: Provider,
        runner: &ProcessRunner,
        cwd: &std::path::Path,
        stop: &CancellationToken,
    ) -> Result<()> {
        let program = match provider {
            Provider::Codex => "codex",
            Provider::Claude => "claude",
        };
        let help_args = if provider == Provider::Codex {
            vec![program.into(), "exec".into(), "--help".into()]
        } else {
            vec![program.into(), "--help".into()]
        };
        let help = runner.run(help_args, cwd, 15, Uuid::new_v4(), stop).await?;
        require_probe_outcome(&help)?;
        if !help.succeeded() {
            return Err(failure("missing_cli", help.summary(), true));
        }
        let help = help.stdout_text()?;
        let flags = if provider == Provider::Codex {
            vec![
                "--json",
                "--sandbox",
                "--output-schema",
                "--ignore-user-config",
            ]
        } else {
            vec![
                "--print",
                "stream-json",
                "--json-schema",
                "--permission-prompts",
                "--safe-mode",
                "--tools",
                "--strict-mcp-config",
            ]
        };
        for flag in flags {
            if !help.contains(flag) {
                return Err(failure(
                    "unsupported_cli",
                    format!(
                        "{program} does not advertise {flag}; update CLI or choose another provider"
                    ),
                    true,
                ));
            }
        }
        if provider == Provider::Codex {
            // Avoid the shared app-server so Runtime v0 owns this invocation's process tree.
            let global = runner
                .run(
                    vec![program.into(), "--help".into()],
                    cwd,
                    15,
                    Uuid::new_v4(),
                    stop,
                )
                .await?;
            require_probe_outcome(&global)?;
            if !global.succeeded() || !global.stdout_text()?.contains("--no-daemon") {
                return Err(failure(
                    "unsupported_cli",
                    "Codex must support --no-daemon for local process ownership",
                    true,
                ));
            }
        }
        Ok(())
    }
}

fn require_probe_outcome(output: &CommandOutput) -> Result<()> {
    use super::VerificationOutcome;
    match output.outcome() {
        VerificationOutcome::Timeout => Err(failure("timeout", output.summary(), false)),
        VerificationOutcome::Cancelled => Err(failure("cancelled", output.summary(), false)),
        // A missing executable is a known runtime spawn failure, not a transcript claim.
        VerificationOutcome::InfrastructureFailure => {
            Err(failure("cli_infrastructure", output.summary(), true))
        }
        _ => Ok(()),
    }
}

/// Decode only documented JSONL envelopes; terminal scraping is never a success path.
pub fn decode(provider: Provider, output: &CommandOutput) -> Result<AgentResponse> {
    use super::VerificationOutcome;
    match output.outcome() {
        VerificationOutcome::Timeout => return Err(failure("timeout", output.summary(), false)),
        VerificationOutcome::Cancelled => {
            return Err(failure("cancelled", output.summary(), false));
        }
        VerificationOutcome::InfrastructureFailure => {
            return Err(failure("infrastructure_failure", output.summary(), false));
        }
        _ => {}
    }
    if output.truncated {
        return Err(failure(
            "output_limit",
            "agent stream exceeded 2 MiB; complete output retained in process events",
            false,
        ));
    }
    let stdout = std::str::from_utf8(&output.stdout).context("agent emitted non-UTF8 JSONL")?;
    let mut session = None;
    let mut result = None;
    let mut complete = false;
    let mut errors = Vec::new();
    let mut denied = false;
    let mut credentials = false;
    for line in stdout.lines().filter(|s| !s.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).context("invalid agent JSONL event")?;
        // Only provider-owned envelope fields classify blockers. Tool output,
        // transcript text and diagnostic strings never grant this authority.
        let envelope = matches!(
            event["type"].as_str(),
            Some("error" | "turn.failed" | "result")
        ) || (provider == Provider::Claude
            && event["type"] == "assistant"
            && event.get("error").is_some());
        if envelope {
            let error = &event["error"];
            let code = error["code"]
                .as_str()
                .or(error["type"].as_str())
                .or(error.as_str());
            credentials |= matches!(
                code,
                Some(
                    "authentication_error"
                        | "authentication_failed"
                        | "unauthorized"
                        | "invalid_api_key"
                        | "login_required"
                )
            ) || error["status_code"] == 401
                || error["http_status_code"] == 401;
            denied |= matches!(
                code,
                Some("permission_denied" | "approval_required" | "sandbox_denied")
            );
        }
        match provider {
            Provider::Codex => match event["type"].as_str().unwrap_or("") {
                "thread.started" => {
                    session = event["thread_id"].as_str().map(String::from);
                }
                "item.completed" if event["item"]["type"] == "agent_message" => {
                    result = event["item"]["text"].as_str().map(String::from);
                }
                "turn.completed" => {
                    complete = true;
                }
                "turn.failed" | "error" => {
                    errors.push(event.to_string());
                }
                _ => {}
            },
            Provider::Claude => {
                if let Some(id) = event["session_id"].as_str() {
                    session = Some(id.into());
                }
                if event["subtype"] == "permission_denied" {
                    denied = true;
                }
                if event["type"] == "result" {
                    denied |= event["permission_denials"]
                        .as_array()
                        .is_some_and(|v| !v.is_empty());
                    complete = event["subtype"] == "success" && event["is_error"] != true;
                    if !complete {
                        errors.push(event.to_string());
                    }
                    if let Some(value) = event.get("structured_output") {
                        result = Some(serde_json::to_string(value)?);
                    }
                }
            }
        }
    }
    if credentials {
        return Err(failure(
            "credentials_required",
            "provider reported a structured authentication failure",
            true,
        ));
    }
    if denied {
        return Err(failure(
            "approval_required",
            "provider denied an operation requiring human permission; no interactive host is attached",
            true,
        ));
    }
    if !output.succeeded() || !complete || !errors.is_empty() {
        let diagnostic = format!("{} {}", errors.join(" "), output.summary());
        return Err(failure(
            "provider_failure",
            diagnostic.chars().take(4000).collect::<String>(),
            false,
        ));
    }
    if session.as_ref().is_none_or(|s| Uuid::parse_str(s).is_err()) {
        return Err(failure(
            "protocol_error",
            "provider emitted an invalid session UUID",
            false,
        ));
    }
    let result: Value =
        serde_json::from_str(&result.context("agent completed without structured result")?)
            .context("invalid structured agent result")?;
    Ok(AgentResponse {
        session_id: session,
        result,
    })
}

fn object(properties: Value) -> Value {
    let required: Vec<_> = properties
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}
fn string() -> Value {
    json!({"type":"string"})
}
fn strings() -> Value {
    json!({"type":"array","items":{"type":"string"}})
}
pub fn schema(role: AgentRole) -> Value {
    match role {
        AgentRole::Coder => object(json!({"summary":string(), "blocker":string()})),
        AgentRole::Planner => object(json!({
            "human_requirements":{"type":"array","items":object(json!({"kind":{"type":"string","enum":["environment","product_decision","permission"]},"reason":string(),"requested_action":string(),"affected_tasks":strings(),"blocking":{"type":"boolean"},"evidence":string(),"confidence":{"type":"integer","minimum":0,"maximum":100},"permission":{"anyOf":[{"type":"null"},object(json!({"operation":{"type":"string","enum":["workspace_write","run_checks","outside_workspace","protected_configuration","install_dependencies","environment_change","destructive_migration","external_operation"]},"scope":string()}))]}}))},
            "requirements":{"type":"array","items":object(json!({"id":string(),"text":string()}))},
            "milestones":{"type":"array","items":object(json!({"id":string(),"goal":string(),"task_ids":strings()}))},
            "tasks":{"type":"array","items":object(json!({"id":string(),"goal":string(),"description":string(),"acceptance_criteria":strings(),"dependencies":strings(),"requirement_ids":strings(),"verification_expectations":strings(),"role":{"type":"string","enum":["coder"]}}))}
        })),
        AgentRole::Reviewer => object(
            json!({"accepted":{"type":"boolean"}, "blocker":string(), "criteria":{"type":"array","items":object(json!({"criterion_id":string(),"criterion":string(),"satisfied":{"type":"boolean"},"evidence":string()}))}, "findings":{"type":"array","items":object(json!({"severity":{"type":"string","enum":["critical","high","medium","low","info"]},"file":string(),"line":{"type":"integer","minimum":0},"explanation":string(),"recommended_fix":string(),"confidence":{"type":"number","minimum":0,"maximum":1}}))}}),
        ),
    }
}

pub(super) fn review_schema(task: &super::PlannedTask) -> Value {
    let mut schema = schema(AgentRole::Reviewer);
    schema["properties"]["criteria"]["items"]["properties"]["criterion_id"] =
        json!({"type":"string", "enum":task.criterion_ids()});
    schema
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Run, RunSpec, RunStatus};
    fn output(stdout: String, stderr: &str, status: RunStatus) -> CommandOutput {
        let now = chrono::Utc::now();
        CommandOutput {
            run: Run {
                id: Uuid::new_v4(),
                output_complete: Some(true),
                submission_key: None,
                cleanup_warnings: vec![],
                created_at: now,
                updated_at: now,
                status,
                spec: RunSpec {
                    command: vec!["fake-agent".into()],
                    cwd: std::env::temp_dir(),
                    env: Default::default(),
                    timeout_seconds: Some(1),
                    keep_awake: false,
                },
                pid: None,
                started_at: Some(now),
                finished_at: Some(now),
                exit_code: Some(if status == RunStatus::Succeeded { 0 } else { 1 }),
                detail: None,
            },
            stdout: stdout.into_bytes(),
            stderr: stderr.as_bytes().to_vec(),
            truncated: false,
        }
    }
    #[test]
    fn provider_envelopes_require_structured_completion_and_classify_blockers() -> Result<()> {
        let id = Uuid::new_v4();
        let codex = format!(
            "{}\n{}\n{}\n",
            json!({"type":"thread.started","thread_id":id}),
            json!({"type":"item.completed","item":{"type":"agent_message","text":"{\"summary\":\"done\",\"blocker\":\"\"}"}}),
            json!({"type":"turn.completed"})
        );
        assert_eq!(
            decode(Provider::Codex, &output(codex, "", RunStatus::Succeeded))?.session_id,
            Some(id.to_string())
        );
        let claude=json!({"type":"result","subtype":"success","session_id":id,"structured_output":{"summary":"done","blocker":""}}).to_string();
        assert_eq!(
            decode(Provider::Claude, &output(claude, "", RunStatus::Succeeded))?.result["summary"],
            "done"
        );
        for stdout in [
            String::new(),
            "not JSON".into(),
            json!({"type":"turn.completed"}).to_string(),
            json!({"type":"turn.failed","error":{"message":"failed"}}).to_string(),
        ] {
            assert!(decode(Provider::Codex, &output(stdout, "", RunStatus::Succeeded)).is_err());
        }
        let denied=json!({"type":"result","subtype":"success","session_id":id,"structured_output":{},"permission_denials":[{"tool_name":"Edit"}]}).to_string();
        let error = decode(Provider::Claude, &output(denied, "", RunStatus::Succeeded))
            .expect_err("permission denial must block");
        let error = error.downcast_ref::<AgentFailure>().expect("classified");
        assert!(error.blocked);
        assert_eq!(error.kind, "approval_required");
        let error = decode(
            Provider::Claude,
            &output(
                json!({"type":"error","error":{"type":"authentication_error","message":"login failed"}}).to_string(),
                "",
                RunStatus::Failed,
            ),
        )
        .expect_err("authentication must block");
        assert_eq!(
            error
                .downcast_ref::<AgentFailure>()
                .expect("classified")
                .kind,
            "credentials_required"
        );
        Ok(())
    }
    #[test]
    fn transcript_words_cannot_classify_blockers_and_runtime_outcomes_take_priority() -> Result<()>
    {
        let misleading = "authentication approval sandbox timeout permission denied credentials";
        for provider in [Provider::Codex, Provider::Claude] {
            let events = if provider == Provider::Codex {
                json!({"type":"turn.failed","error":{"message":misleading}}).to_string()
            } else {
                json!({"type":"result","subtype":"error_during_execution","errors":[misleading]})
                    .to_string()
            };
            let e = decode(
                provider,
                &output(events.clone(), misleading, RunStatus::Failed),
            )
            .expect_err("provider failure");
            let f = e.downcast_ref::<AgentFailure>().expect("classified");
            assert_eq!(f.kind, "provider_failure");
            assert!(!f.blocked);
            let mut timed = output(events, misleading, RunStatus::Failed);
            timed.run.detail = Some("timeout exceeded".into());
            timed.run.exit_code = None;
            assert_eq!(
                decode(provider, &timed)
                    .expect_err("timeout")
                    .downcast_ref::<AgentFailure>()
                    .expect("classified")
                    .kind,
                "timeout"
            );
            timed.run.status = RunStatus::Cancelled;
            assert_eq!(
                decode(provider, &timed)
                    .expect_err("cancelled")
                    .downcast_ref::<AgentFailure>()
                    .expect("classified")
                    .kind,
                "cancelled"
            );
            let auth = if provider == Provider::Codex {
                json!({"type":"turn.failed","error":{"code":"unauthorized"}})
            } else {
                json!({"type":"error","error":{"type":"authentication_error"}})
            };
            let e = decode(provider, &output(auth.to_string(), "", RunStatus::Failed))
                .expect_err("structured auth");
            assert!(
                e.downcast_ref::<AgentFailure>()
                    .expect("classified")
                    .blocked
            );
        }
        Ok(())
    }
}
