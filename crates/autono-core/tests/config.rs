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
