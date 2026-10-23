mod client;
mod doctor;
mod engineering;

use std::{collections::BTreeMap, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use autono_core::config::{Config, Paths};
use autono_protocol::{Event, EventKind, OutputStream, Run, RunSpec, RunView, Submission};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use client::Client;

#[derive(Parser)]
#[command(
    name = "autono",
    version,
    about = "Local-first durable process runtime"
)]
struct Cli {
    /// Override user data/config directory (also AUTONO_HOME); must be absolute.
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Plan a Markdown feature in a retained read-only checkout; returns immediately.
    Plan {
        specification: PathBuf,
        #[arg(long)]
        repository: Option<PathBuf>,
        #[arg(long, default_value = "HEAD")]
        base: String,
        /// Reuse this UUID after an ambiguous submission response.
        #[arg(long)]
        plan_id: Option<Uuid>,
    },
    /// Inspect a bounded autonomous contract without invoking any agent. Save stdout for consent.
    ProposeDelegation {
        specification: PathBuf,
        #[arg(long)]
        repository: Option<PathBuf>,
        #[arg(long, default_value = "HEAD")]
        base: String,
        #[arg(long)]
        plan_id: Option<Uuid>,
        #[arg(long = "allow-file", required = true)]
        writable_files: Vec<String>,
        #[arg(long, default_value_t = 1800)]
        max_runtime_seconds: u64,
        #[arg(long)]
        local_integration: bool,
    },
    /// Consent to exactly the previously inspected JSON contract; starts one unattended run.
    AuthorizeDelegation { contract: PathBuf },
    /// Inspect the durable delegation proposal (summary also includes recorded consent).
    ShowDelegation { id: Uuid },
    /// Inspect the immutable plan, original specification and configured checks.
    ShowPlan { id: Uuid },
    /// Approve the persisted plan and start daemon-owned task execution.
    Execute { id: Uuid },
    /// Refresh durable host checks and inspect preflight readiness.
    Preflight { id: Uuid },
    /// List durable human actions, resolutions and audit history.
    Blockers { id: Uuid },
    /// Resolve a human action using a JSON request file (never include credentials).
    Resolve {
        id: Uuid,
        blocker_id: String,
        #[arg(long)]
        request: PathBuf,
    },
    /// Continue eligible explicitly resolved live pauses; never resumes interrupted runs.
    Continue { id: Uuid },
    /// Inspect engineering task progress and retained artifacts.
    Tasks { id: Uuid },
    /// Inspect independent review findings and manual blockers.
    Findings { id: Uuid },
    /// Inspect decisions, evidence and append-only review history for a run.
    Decisions {
        id: Uuid,
        #[arg(long)]
        json: bool,
    },
    /// Merge verified task artifacts into a local Autono integration branch and check it.
    Integrate { id: Uuid },
    /// Show a detailed engineering run summary.
    Summary { id: Uuid },
    /// Read the evidence-based partial completion and exact remaining prerequisites.
    Handoff {
        id: Uuid,
        #[arg(long)]
        json: bool,
    },
    /// Inspect optional tooling, repository, daemon, database, and host capabilities.
    Doctor,
    /// Run the daemon in the foreground until Ctrl-C or SIGTERM.
    Daemon,
    /// Submit a command and print its durable run ID. Does not wait for completion.
    Run {
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long = "env", value_parser = parse_env)]
        env: Vec<(String, String)>,
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(long)]
        no_keep_awake: bool,
        /// Reuse this UUID to retry an ambiguous submission safely.
        #[arg(long)]
        submission_key: Option<Uuid>,
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
    /// List recent runs or inspect one run.
    Status {
        /// Inspect engineering workflows instead of process runs.
        #[arg(long)]
        engineering: bool,
        id: Option<Uuid>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    /// Replay persisted stdout/stderr; optionally follow until the run finishes.
    Logs {
        /// Replay durable engineering history and evidence references.
        #[arg(long)]
        engineering: bool,
        id: Uuid,
        #[arg(short, long)]
        follow: bool,
        /// Print structured events rather than raw process output.
        #[arg(long)]
        events: bool,
        /// Resume after this durable sequence number.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..))]
        after: i64,
    },
    /// Request cancellation of an owned run (completion is asynchronous).
    Cancel {
        id: Uuid,
        #[arg(long)]
        engineering: bool,
    },
}

fn parse_env(value: &str) -> Result<(String, String), String> {
    let (key, value) = value.split_once('=').ok_or("expected KEY=VALUE")?;
    if key.is_empty() || key.contains('\0') || value.contains('\0') {
        return Err("invalid environment variable".into());
    }
    Ok((key.into(), value.into()))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    let paths = Paths::discover(cli.home)?;
    match cli.command {
        Commands::Plan {
            specification,
            repository,
            base,
            plan_id,
        } => engineering::plan(&paths, specification, repository, base, plan_id).await,
        Commands::ProposeDelegation {
            specification,
            repository,
            base,
            plan_id,
            writable_files,
            max_runtime_seconds,
            local_integration,
        } => {
            engineering::propose_delegation(
                &paths,
                specification,
                autono_protocol::engineering::DelegationProposalRequest {
                    submission: autono_protocol::engineering::PlanSubmission {
                        id: plan_id.unwrap_or_else(Uuid::new_v4),
                        repository: repository.unwrap_or(std::env::current_dir()?),
                        base,
                        specification: String::new(),
                    },
                    writable_files,
                    max_runtime_seconds,
                    local_integration,
                },
            )
            .await
        }
        Commands::AuthorizeDelegation { contract } => {
            engineering::authorize_delegation(&paths, contract).await
        }
        Commands::ShowDelegation { id } => engineering::show_delegation(&paths, id).await,
        Commands::ShowPlan { id } => engineering::inspect(&paths, id, "plan").await,
        Commands::Tasks { id } => engineering::inspect(&paths, id, "tasks").await,
        Commands::Decisions { id, json } => engineering::decisions(&paths, id, json).await,
        Commands::Findings { id } => engineering::inspect(&paths, id, "findings").await,
        Commands::Handoff { id, json } => engineering::handoff(&paths, id, json).await,
        Commands::Summary { id } => engineering::inspect(&paths, id, "summary").await,
        Commands::Preflight { id } => engineering::preflight(&paths, id).await,
        Commands::Blockers { id } => engineering::inspect(&paths, id, "blockers").await,
        Commands::Resolve {
            id,
            blocker_id,
            request,
        } => engineering::resolve(&paths, id, &blocker_id, request).await,
        Commands::Continue { id } => engineering::action(&paths, id, "continue").await,
        Commands::Execute { id } => engineering::action(&paths, id, "execute").await,
        Commands::Integrate { id } => engineering::action(&paths, id, "integrate").await,
        Commands::Doctor => doctor::run(&paths).await,
        Commands::Daemon => {
            let config = Config::load(&paths).await?;
            let stop = CancellationToken::new();
            let signal = stop.clone();
            tokio::spawn(async move {
                if let Err(error) = autono_daemon::shutdown_signal().await {
                    eprintln!("signal handler failed: {error:#}");
                }
                signal.cancel();
            });
            autono_daemon::run(paths, config, stop).await
        }
        Commands::Run {
            cwd,
            env,
            timeout,
            no_keep_awake,
            submission_key,
            command,
        } => {
            let cwd = tokio::fs::canonicalize(cwd.unwrap_or(std::env::current_dir()?))
                .await
                .context("resolve working directory")?;
            let config = Config::load(&paths).await?.with_project(&cwd).await?;
            let spec = RunSpec {
                command,
                cwd,
                env: env.into_iter().collect::<BTreeMap<_, _>>(),
                timeout_seconds: timeout,
                keep_awake: config.runner.keep_awake && !no_keep_awake,
            };
            spec.validate()?;
            let submission = Submission {
                submission_key: submission_key.unwrap_or_else(Uuid::new_v4),
                spec,
            };
            eprintln!("submission key: {}", submission.submission_key);
            let run = Client::connect(&paths).await?.submit(&submission).await?;
            println!("{}", run.id);
            Ok(())
        }
        Commands::Status {
            engineering: workflow,
            id,
            json,
            limit,
            offset,
        } => {
            if workflow {
                return engineering::status(&paths, id, json, limit, offset).await;
            }
            let client = Client::connect(&paths).await?;
            if let Some(id) = id {
                let run: RunView = client.get(&format!("/v1/runs/{id}")).await?;
                println!("{}", serde_json::to_string_pretty(&run)?);
            } else {
                let runs: Vec<RunView> = client
                    .get(&format!("/v1/runs?limit={limit}&offset={offset}"))
                    .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&runs)?);
                } else {
                    println!("{:<36}  {:<11}  COMMAND", "ID", "STATUS");
                    for view in runs {
                        let status = view
                            .runtime
                            .as_ref()
                            .map(|owner| owner.phase.as_str())
                            .unwrap_or_else(|| view.run.status.as_str());
                        let run = view.run;
                        println!(
                            "{}  {:<11}  {}",
                            run.id,
                            status,
                            serde_json::to_string(&run.spec.command)?
                        );
                    }
                }
            }
            Ok(())
        }
        Commands::Logs {
            engineering: workflow,
            id,
            follow,
            events,
            after,
        } => {
            if workflow {
                return engineering::logs(&paths, id, after, follow).await;
            }
            let mut client = Client::connect(&paths).await?;
            let _: Run = client.get(&format!("/v1/runs/{id}")).await?;
            if follow {
                client
                    .follow(id, after, |event| print_event(event, events))
                    .await?;
            } else {
                let mut after = after;
                loop {
                    let batch: Vec<Event> = client
                        .get(&format!("/v1/runs/{id}/events?after={after}"))
                        .await?;
                    if batch.is_empty() {
                        break;
                    }
                    for event in batch {
                        after = event.sequence;
                        print_event(event, events)?;
                    }
                }
            }
            Ok(())
        }
        Commands::Cancel {
            id,
            engineering: workflow,
        } => {
            if workflow {
                return engineering::action(&paths, id, "cancel").await;
            }
            let _: autono_protocol::CancelResponse = Client::connect(&paths)
                .await?
                .post(&format!("/v1/runs/{id}/cancel"), &())
                .await?;
            println!("cancellation requested for {id}");
            Ok(())
        }
    }
}

fn print_event(event: Event, structured: bool) -> Result<()> {
    if structured {
        println!("{}", serde_json::to_string(&event)?);
    } else if let EventKind::Output { stream, data } = event.kind {
        let bytes = STANDARD.decode(data).context("decode persisted output")?;
        match stream {
            OutputStream::Stdout => {
                let mut out = std::io::stdout().lock();
                out.write_all(&bytes)?;
                out.flush()?;
            }
            OutputStream::Stderr => {
                let mut out = std::io::stderr().lock();
                out.write_all(&bytes)?;
                out.flush()?;
            }
        }
    }
    Ok(())
}
