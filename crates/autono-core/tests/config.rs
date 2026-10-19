use anyhow::Result;
use autono_core::config::{Config, Paths};

#[tokio::test]
async fn user_and_project_config_merge_without_resetting_unrelated_settings() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let paths = Paths {
        home: dir.path().join("home"),
    };
    paths.prepare().await?;
    assert!(Config::load(&paths).await?.runner.keep_awake);
    tokio::fs::write(
        paths.config(),
        "[runner]\nkeep_awake = false\n[daemon]\nport = 4567\n",
    )
    .await?;
    let config = Config::load(&paths).await?.with_project(dir.path()).await?;
    assert!(!config.runner.keep_awake);
    assert_eq!(config.daemon.port, 4567);
    tokio::fs::write(
        dir.path().join("autono.toml"),
        "[runner]\nkeep_awake = true\n",
    )
    .await?;
    let config = Config::load(&paths).await?.with_project(dir.path()).await?;
    assert!(config.runner.keep_awake);
    assert_eq!(config.daemon.port, 4567);
    tokio::fs::write(dir.path().join("autono.toml"), "[daemon]\nport = 9999\n").await?;
    assert!(config.with_project(dir.path()).await.is_err());
    tokio::fs::write(paths.config(), "[runner]\nkeep_awke = true\n").await?;
    assert!(Config::load(&paths).await.is_err());
    Ok(())
}

#[tokio::test]
async fn inspecting_a_missing_database_does_not_create_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("missing.sqlite3");
    assert!(autono_core::Store::inspect(&database).await.is_err());
    assert!(!tokio::fs::try_exists(database).await?);
    Ok(())
}

#[tokio::test]
async fn engineering_config_merges_fields_and_validates_limits() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let paths = Paths {
        home: dir.path().join("home"),
    };
    paths.prepare().await?;
    tokio::fs::write(paths.config(),"[agents.coder]\nprovider='claude'\nmodel='configured-model'\n[execution]\nmax_attempts=4\n[verification]\ncommands=[['sh','check.sh']]\ntimeout_seconds=15\n").await?;
    tokio::fs::write(dir.path().join("autono.toml"),"[agents.reviewer]\nprovider='codex'\n[execution]\nmax_parallel=1\n[verification]\ntimeout_seconds=20\n").await?;
    let config = Config::load(&paths).await?.with_project(dir.path()).await?;
    assert_eq!(
        config.agents.coder.provider,
        autono_protocol::engineering::Provider::Claude
    );
    assert_eq!(
        config.agents.coder.model.as_deref(),
        Some("configured-model")
    );
    assert_eq!(config.execution.max_attempts, 4);
    assert_eq!(config.execution.max_parallel, 1);
    assert_eq!(config.verification.commands, vec![vec!["sh", "check.sh"]]);
    assert_eq!(config.verification.timeout_seconds, 20);
    tokio::fs::write(paths.config(), "[execution]\nmax_review_cycles=0\n").await?;
    assert!(Config::load(&paths).await.is_err());
    Ok(())
}
