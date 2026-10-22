use crate::client::Client;
use anyhow::{Context, Result};
use autono_core::config::Paths;
use autono_protocol::engineering::{EngineeringEvent, EngineeringRun, PlanSubmission};
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

pub async fn plan(
    paths: &Paths,
    specification: PathBuf,
    repository: Option<PathBuf>,
    base: String,
    id: Option<Uuid>,
) -> Result<()> {
    let repository = tokio::fs::canonicalize(repository.unwrap_or(std::env::current_dir()?))
        .await
        .context("resolve repository")?;
    let bytes = tokio::fs::read(specification)
        .await
        .context("read Markdown feature specification")?;
    anyhow::ensure!(bytes.len() <= 128 * 1024, "specification exceeds 128 KiB");
    let submission = PlanSubmission {
        id: id.unwrap_or_else(Uuid::new_v4),
        repository,
        base,
        specification: String::from_utf8(bytes).context("specification must be UTF-8")?,
    };
    eprintln!(
        "plan ID: {} (reuse --plan-id after an ambiguous response)",
        submission.id
    );
    let run: EngineeringRun = Client::connect(paths)
        .await?
        .post("/v1/engineering", &submission)
        .await?;
    println!("{}", run.id);
    Ok(())
}
pub async fn action(paths: &Paths, id: Uuid, action: &str) -> Result<()> {
    let result: serde_json::Value = Client::connect(paths)
        .await?
        .post(&format!("/v1/engineering/{id}/{action}"), &())
        .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"id":id,"action":action,"status":result.get("status"),"accepted":result.get("accepted")})
        )?
    );
    Ok(())
}
pub async fn inspect(paths: &Paths, id: Uuid, view: &str) -> Result<()> {
    let run: EngineeringRun = Client::connect(paths)
        .await?
        .get(&format!("/v1/engineering/{id}"))
        .await?;
    let value = match view {
        "plan" => {
            json!({"id":run.id,"status":run.status,"specification":run.specification,"base_commit":run.base_commit,"settings":run.settings,"plan":run.plan,"approved_at":run.approved_at})
        }
        "tasks" => json!(run.tasks),
        "blockers" => json!(run.human_actions),
        "findings" => {
            json!({"blockers":run.blockers,"reviews":run.tasks.iter().map(|t|json!({"task":t.plan.id,"attempts":t.attempts.iter().map(|a|json!({"attempt":a.number,"reviewer":a.reviewer,"review":a.review,"detail":a.detail})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
        }
        _ => serde_json::to_value(run)?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
pub async fn status(
    paths: &Paths,
    id: Option<Uuid>,
    json_output: bool,
    limit: u32,
    offset: u32,
) -> Result<()> {
    if let Some(id) = id {
        return inspect(paths, id, "summary").await;
    }
    let runs: Vec<EngineeringRun> = Client::connect(paths)
        .await?
        .get(&format!("/v1/engineering?limit={limit}&offset={offset}"))
        .await?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&runs)?);
    } else {
        println!("{:<36}  {:<18}  TASKS  SUMMARY", "ID", "STATUS");
        for run in runs {
            println!(
                "{}  {:<18}  {:>2}/{:<2}  {}",
                run.id,
                format!("{:?}", run.status),
                run.tasks
                    .iter()
                    .filter(|t| t.status == autono_protocol::engineering::TaskStatus::Completed)
                    .count(),
                run.tasks.len(),
                run.summary
            );
        }
    }
    Ok(())
}
pub async fn logs(paths: &Paths, id: Uuid, mut after: i64, follow: bool) -> Result<()> {
    let client = Client::connect(paths).await?;
    loop {
        let batch: Vec<EngineeringEvent> = client
            .get(&format!("/v1/engineering/{id}/events?after={after}"))
            .await?;
        if !batch.is_empty() {
            for event in batch {
                after = event.sequence;
                println!("{}", serde_json::to_string(&event)?);
            }
            continue;
        }
        if !follow {
            break;
        }
        let run: EngineeringRun = client.get(&format!("/v1/engineering/{id}")).await?;
        if !run.status.active() {
            let tail: Vec<EngineeringEvent> = client
                .get(&format!("/v1/engineering/{id}/events?after={after}"))
                .await?;
            for event in tail {
                println!("{}", serde_json::to_string(&event)?);
            }
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(())
}

pub async fn preflight(paths: &Paths, id: Uuid) -> Result<()> {
    let run: EngineeringRun = Client::connect(paths)
        .await?
        .post(&format!("/v1/engineering/{id}/preflight"), &())
        .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"status": if run.preflight.as_ref().is_some_and(|p| p.ready) {"READY"} else {"NOT READY"}, "preflight":run.preflight, "human_actions":run.human_actions, "permissions":run.settings.permissions})
        )?
    );
    Ok(())
}
pub async fn resolve(paths: &Paths, id: Uuid, action: &str, path: PathBuf) -> Result<()> {
    let request: autono_protocol::engineering::ResolutionRequest =
        serde_json::from_slice(&tokio::fs::read(path).await?)
            .context("read resolution JSON (never include credentials)")?;
    let run: EngineeringRun = Client::connect(paths)
        .await?
        .post(
            &format!("/v1/engineering/{id}/blockers/{action}/resolve"),
            &request,
        )
        .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&run.human_actions.iter().find(|a| a.id == action))?
    );
    Ok(())
}

pub async fn propose_delegation(
    paths: &Paths,
    specification: PathBuf,
    mut request: autono_protocol::engineering::DelegationProposalRequest,
) -> Result<()> {
    request.submission.repository = tokio::fs::canonicalize(&request.submission.repository).await?;
    let bytes = tokio::fs::read(specification).await?;
    anyhow::ensure!(bytes.len() <= 128 * 1024, "specification exceeds 128 KiB");
    request.submission.specification = String::from_utf8(bytes)?;
    eprintln!(
        "delegation ID: {} (reuse --plan-id after an ambiguous response)",
        request.submission.id
    );
    let contract: autono_protocol::engineering::DelegationContract = Client::connect(paths)
        .await?
        .post("/v1/delegations/propose", &request)
        .await?;
    println!("{}", serde_json::to_string_pretty(&contract)?);
    Ok(())
}
pub async fn authorize_delegation(paths: &Paths, path: PathBuf) -> Result<()> {
    let contract: autono_protocol::engineering::DelegationContract =
        serde_json::from_slice(&tokio::fs::read(path).await?)?;
    let run: EngineeringRun = Client::connect(paths)
        .await?
        .post(
            &format!(
                "/v1/delegations/{}/authorize",
                contract.request.submission.id
            ),
            &contract,
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&run)?);
    Ok(())
}
pub async fn show_delegation(paths: &Paths, id: Uuid) -> Result<()> {
    let contract: autono_protocol::engineering::DelegationContract = Client::connect(paths)
        .await?
        .get(&format!("/v1/delegations/{id}"))
        .await?;
    println!("{}", serde_json::to_string_pretty(&contract)?);
    Ok(())
}
