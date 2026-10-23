use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use uuid::Uuid;

use crate::{Event, EventKind, Run, RunSpec, RunStatus};

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
    pub(crate) readers: SqlitePool,
    pub(crate) changed: tokio::sync::watch::Sender<()>,
}

#[derive(Debug, thiserror::Error)]
#[error("run {0} not found")]
pub struct RunNotFound(pub Uuid);

#[derive(Debug, thiserror::Error)]
#[error("illegal transition from {from:?} to {to:?}")]
pub struct InvalidTransition {
    pub from: RunStatus,
    pub to: RunStatus,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self> {
        Self::connect(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true),
        )
        .await
    }

    /// Diagnostic connection: does not create or migrate a missing database.
    pub async fn inspect(path: &Path) -> Result<Self> {
        Self::connect_options(SqliteConnectOptions::new().filename(path).read_only(true)).await
    }

    async fn connect(options: SqliteConnectOptions) -> Result<Self> {
        let store = Self::connect_options(
            options
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Full),
        )
        .await?;
        sqlx::migrate!()
            .run(&store.pool)
            .await
            .context("migrate runtime database")?;
        Ok(store)
    }

    async fn connect_options(options: SqliteConnectOptions) -> Result<Self> {
        // One connection serializes local writes and avoids read-to-write upgrade contention.
        let reader_options = options
            .clone()
            .read_only(true)
            .create_if_missing(false)
            .busy_timeout(Duration::from_millis(100));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(2))
            .connect_with(
                options
                    .foreign_keys(true)
                    .busy_timeout(Duration::from_millis(100)),
            )
            .await
            .context("open runtime database writer")?;
        let readers = connect_readers(
            reader_options,
            SqlitePoolOptions::new()
                .max_connections(4)
                .acquire_timeout(Duration::from_millis(250)),
        )
        .await?;
        Ok(Self {
            readers,
            pool,
            changed: tokio::sync::watch::channel(()).0,
        })
    }

    pub async fn health(&self) -> Result<()> {
        let result: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&self.readers)
            .await?;
        anyhow::ensure!(result == "ok", "SQLite quick_check: {result}");
        let _: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
            .fetch_one(&self.readers)
            .await
            .context("runtime schema missing")?;
        Ok(())
    }

    /// Subscribe before reading to avoid missed wakeups. Events remain the source of truth.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
        self.changed.subscribe()
    }

    pub async fn create(&self, spec: RunSpec) -> Result<Run> {
        Ok(self.create_keyed(uuid::Uuid::new_v4(), spec).await?.0)
    }

    pub async fn find_submission(&self, key: Uuid) -> Result<Option<Run>> {
        let document: Option<String> =
            sqlx::query_scalar("SELECT document FROM runs WHERE submission_key = ?")
                .bind(key.to_string())
                .fetch_optional(&self.readers)
                .await?;
        document
            .map(|doc| serde_json::from_str(&doc).context("decode submission"))
            .transpose()
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.readers).await?;
        Ok(())
    }

    pub async fn create_keyed(&self, key: Uuid, spec: RunSpec) -> Result<(Run, bool)> {
        spec.validate()?;
        let now = Utc::now();
        let run = Run {
            output_complete: None,
            submission_key: Some(key),
            cleanup_warnings: Vec::new(),
            id: Uuid::new_v4(),
            created_at: now,
            updated_at: now,
            status: RunStatus::Pending,
            spec,
            pid: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
            detail: None,
        };
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query("INSERT INTO runs(id,status,document,submission_key) VALUES(?,?,?,?) ON CONFLICT(submission_key) DO NOTHING")
            .bind(run.id.to_string()).bind(run.status.as_str()).bind(serde_json::to_string(&run)?).bind(key.to_string()).execute(&mut *tx).await?.rows_affected();
        if inserted == 0 {
            let doc: String =
                sqlx::query_scalar("SELECT document FROM runs WHERE submission_key = ?")
                    .bind(key.to_string())
                    .fetch_one(&mut *tx)
                    .await?;
            let existing: Run = serde_json::from_str(&doc)?;
            if existing.spec != run.spec {
                return Err(SubmissionConflict.into());
            }
            tx.commit().await?;
            return Ok((existing, false));
        }
        append(&mut tx, run.id, &EventKind::Created).await?;
        tx.commit().await.context("commit new run")?;
        self.changed.send_replace(());
        Ok((run, true))
    }

    pub async fn get(&self, id: Uuid) -> Result<Run> {
        let document: Option<String> = sqlx::query_scalar("SELECT document FROM runs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.readers)
            .await?;
        serde_json::from_str(&document.ok_or(RunNotFound(id))?).context("decode stored run")
    }

    pub async fn list(&self, limit: u32, offset: u32) -> Result<Vec<Run>> {
        let documents: Vec<String> =
            sqlx::query_scalar("SELECT document FROM runs ORDER BY rowid DESC LIMIT ? OFFSET ?")
                .bind(limit.min(500))
                .bind(offset)
                .fetch_all(&self.readers)
                .await?;
        documents
            .into_iter()
            .map(|doc| serde_json::from_str(&doc).context("decode stored run"))
            .collect()
    }

    pub async fn transition(
        &self,
        id: Uuid,
        to: RunStatus,
        pid: Option<u32>,
        exit_code: Option<i32>,
        detail: Option<String>,
    ) -> Result<Run> {
        self.change(id, to, pid, exit_code, detail, None, Vec::new())
            .await
    }

    pub async fn start(&self, id: Uuid, pid: u32, launched: chrono::DateTime<Utc>) -> Result<Run> {
        self.change(
            id,
            RunStatus::Running,
            Some(pid),
            None,
            None,
            Some(launched),
            Vec::new(),
        )
        .await
    }

    pub async fn finish(
        &self,
        id: Uuid,
        to: RunStatus,
        exit_code: Option<i32>,
        detail: Option<String>,
        warnings: Vec<String>,
    ) -> Result<Run> {
        self.change(id, to, None, exit_code, detail, None, warnings)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn change(
        &self,
        id: Uuid,
        to: RunStatus,
        pid: Option<u32>,
        exit_code: Option<i32>,
        detail: Option<String>,
        launched: Option<chrono::DateTime<Utc>>,
        warnings: Vec<String>,
    ) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        let document: Option<String> = sqlx::query_scalar("SELECT document FROM runs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        let mut run: Run = serde_json::from_str(&document.ok_or(RunNotFound(id))?)?;
        let from = run.status;
        if !from.can_transition_to(to) {
            return Err(InvalidTransition { from, to }.into());
        }
        run.status = to;
        run.updated_at = Utc::now();
        if to == RunStatus::Running {
            run.started_at = Some(launched.unwrap_or(run.updated_at));
            run.pid = pid;
        }
        if to.is_terminal() {
            run.finished_at = Some(run.updated_at);
            run.exit_code = exit_code;
        }
        run.detail = detail.clone();
        run.cleanup_warnings.extend(warnings.iter().cloned());
        for message in warnings {
            append(&mut tx, id, &EventKind::Warning { message }).await?;
        }
        sqlx::query("UPDATE runs SET status = ?, document = ? WHERE id = ?")
            .bind(to.as_str())
            .bind(serde_json::to_string(&run)?)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        append(&mut tx, id, &EventKind::StateChanged { from, to, detail }).await?;
        tx.commit()
            .await
            .context("commit run transition and event")?;
        self.changed.send_replace(());
        Ok(run)
    }

    /// Retry-safe terminal commit. A matching already-committed result is a no-op;
    /// ordinary `transition` still rejects every outgoing terminal transition.
    pub(crate) async fn finalize(
        &self,
        id: Uuid,
        pid: Option<u32>,
        launched: Option<chrono::DateTime<Utc>>,
        result: &autono_protocol::ObservedResult,
    ) -> Result<()> {
        anyhow::ensure!(
            result.status.is_terminal(),
            "finalization requires a terminal result"
        );
        let mut tx = self.pool.begin().await?;
        let document: Option<String> = sqlx::query_scalar("SELECT document FROM runs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        let mut run: Run = serde_json::from_str(&document.ok_or(RunNotFound(id))?)?;
        if run.status.is_terminal() {
            anyhow::ensure!(
                run.status == result.status
                    && run.exit_code == result.exit_code
                    && run.detail == result.detail
                    && run.finished_at == Some(result.finished_at)
                    && run.cleanup_warnings == result.cleanup_warnings
                    && run.output_complete == result.output_complete,
                "terminal result conflicts with retained finalization for {id}"
            );
            return Ok(());
        }
        if run.status == RunStatus::Pending
            && let Some(pid) = pid
        {
            run.pid = Some(pid);
            run.started_at = launched;
            append(
                &mut tx,
                id,
                &EventKind::StateChanged {
                    from: RunStatus::Pending,
                    to: RunStatus::Running,
                    detail: None,
                },
            )
            .await?;
            run.status = RunStatus::Running;
        }
        let from = run.status;
        anyhow::ensure!(
            from.can_transition_to(result.status),
            InvalidTransition {
                from,
                to: result.status
            }
        );
        run.status = result.status;
        run.updated_at = Utc::now();
        run.finished_at = Some(result.finished_at);
        run.exit_code = result.exit_code;
        run.detail = result.detail.clone();
        run.cleanup_warnings = result.cleanup_warnings.clone();
        run.output_complete = result.output_complete;
        for message in &result.cleanup_warnings {
            append(
                &mut tx,
                id,
                &EventKind::Warning {
                    message: message.clone(),
                },
            )
            .await?;
        }
        sqlx::query("UPDATE runs SET status = ?, document = ? WHERE id = ?")
            .bind(run.status.as_str())
            .bind(serde_json::to_string(&run)?)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        append(
            &mut tx,
            id,
            &EventKind::StateChanged {
                from,
                to: result.status,
                detail: result.detail.clone(),
            },
        )
        .await?;
        tx.commit()
            .await
            .context("commit retained terminal result and events")?;
        self.changed.send_replace(());
        Ok(())
    }

    pub async fn record(&self, id: Uuid, kind: EventKind) -> Result<()> {
        if !matches!(kind, EventKind::Output { .. } | EventKind::Warning { .. }) {
            bail!("lifecycle events must be recorded with state changes");
        }
        let mut tx = self.pool.begin().await?;
        let status: Option<String> = sqlx::query_scalar("SELECT status FROM runs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        anyhow::ensure!(
            status.ok_or(RunNotFound(id))? == "RUNNING",
            "cannot append output/warnings to an inactive run"
        );
        append(&mut tx, id, &kind).await?;
        tx.commit().await?;
        self.changed.send_replace(());
        Ok(())
    }

    pub async fn events(&self, id: Uuid, after: i64, limit: u32) -> Result<Vec<Event>> {
        let rows = sqlx::query("SELECT sequence,timestamp,payload FROM events WHERE run_id = ? AND sequence > ? ORDER BY sequence LIMIT ?")
            .bind(id.to_string()).bind(after).bind(limit.min(500)).fetch_all(&self.readers).await?;
        rows.into_iter()
            .map(|row| {
                Ok(Event {
                    sequence: row.try_get("sequence")?,
                    run_id: id,
                    timestamp: row.try_get::<String, _>("timestamp")?.parse()?,
                    kind: serde_json::from_str(&row.try_get::<String, _>("payload")?)?,
                })
            })
            .collect()
    }

    /// Only called after acquiring the exclusive daemon lock. Never signal a stored PID.
    pub async fn reconcile(&self) -> Result<usize> {
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM runs WHERE status IN ('PENDING','RUNNING')")
                .fetch_all(&self.readers)
                .await?;
        for id in &ids {
            self.transition(
                id.parse()?,
                RunStatus::Interrupted,
                None,
                None,
                Some("daemon restarted; original process ownership cannot be recovered".into()),
            )
            .await?;
        }
        Ok(ids.len())
    }
}

/// Initial read-only handshakes can briefly miss the normal 250ms pool budget
/// on a busy host. Retry only known transient connection errors, with the same
/// bounded persistence policy. No migration, write or process is replayed here.
async fn connect_readers(
    options: SqliteConnectOptions,
    builder: SqlitePoolOptions,
) -> Result<SqlitePool> {
    crate::persistence::retry(|| {
        let options = options.clone();
        let builder = builder.clone();
        async move { Ok(builder.connect_with(options).await?) }
    })
    .await
    .context("open runtime database readers")
}

#[cfg(test)]
mod reader_startup_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[tokio::test]
    async fn transient_initial_reader_handshake_timeout_does_not_prevent_startup() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("runtime.sqlite3");
        let store = Store::open(&path).await?;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let readers = connect_readers(
            SqliteConnectOptions::new().filename(&path).read_only(true),
            SqlitePoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_millis(250))
                .after_connect(move |_, _| {
                    let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
                    Box::pin(async move {
                        if first {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                        Ok(())
                    })
                }),
        )
        .await?;
        assert!(calls.load(Ordering::SeqCst) >= 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM engineering_runs")
                .fetch_one(&readers)
                .await?,
            0
        );
        store.health().await?;
        readers.close().await;
        Ok(())
    }
}

async fn append(tx: &mut Transaction<'_, Sqlite>, id: Uuid, kind: &EventKind) -> Result<()> {
    sqlx::query("INSERT INTO events(run_id,timestamp,payload) VALUES(?,?,?)")
        .bind(id.to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(serde_json::to_string(kind)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("submission key already exists with a different payload")]
pub struct SubmissionConflict;

#[cfg(test)]
mod finalization_tests {
    use super::*;
    use autono_protocol::ObservedResult;

    #[tokio::test]
    async fn repeated_matching_terminal_commit_is_a_noop_and_conflicts_are_rejected() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("runtime.sqlite3")).await?;
        let run = store
            .create(RunSpec {
                command: vec!["unused".into()],
                cwd: std::env::current_dir()?,
                env: Default::default(),
                timeout_seconds: None,
                keep_awake: false,
            })
            .await?;
        let now = Utc::now();
        let observed = ObservedResult {
            status: RunStatus::Failed,
            exit_code: Some(42),
            detail: Some("observed failure".into()),
            cleanup_warnings: vec!["cleanup uncertainty".into()],
            output_complete: Some(false),
            finished_at: now,
        };
        for _ in 0..4 {
            store
                .finalize(run.id, Some(123), Some(now), &observed)
                .await?;
        }
        let events = store.events(run.id, 0, 500).await?;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    EventKind::StateChanged {
                        to: RunStatus::Failed,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::Warning { .. }))
                .count(),
            1
        );
        let mut conflict = observed;
        conflict.exit_code = Some(43);
        assert!(
            store
                .finalize(run.id, Some(123), Some(now), &conflict)
                .await
                .is_err()
        );
        assert!(
            store
                .transition(run.id, RunStatus::Failed, None, Some(42), None)
                .await
                .is_err()
        );
        assert_eq!(store.events(run.id, 0, 500).await?.len(), events.len());
        Ok(())
    }
}
