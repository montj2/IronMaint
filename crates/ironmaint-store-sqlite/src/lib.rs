//! SQLite backend for [`IronMaintStore`].
//!
//! See `README.md` for the runtime-SQL rationale and PRAGMA table.
//!
//! This crate is intentionally small and direct: a single
//! `SqliteStore` struct that owns the `sqlx::SqlitePool` and the
//! exclusive `fs2` lock file. The nine sub-traits from
//! `ironmaint-store` are implemented against the pool one table
//! per trait.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::map_err_ignore,
    )
)]
#![allow(async_fn_in_trait)]
// `sqlx::query`/`query_as` reject dynamic SQL strings. Our queries
// are all static; `raw_sql` and `AssertSqlSafe` only appear in the
// migration-applier where the input is committed `.sql` files.
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Executor, SqlitePool};
use time::OffsetDateTime;

use ironmaint_store::{
    ArtifactMetadataStore, CandidateStore, EventStore, EvidenceStore, GateStore, ObligationStore,
    OperationStore, ProjectionStore, StoreError, StoreErrorKind, WorkspaceMetadataStore,
};

mod config;
mod lock;
mod ops;

pub use config::SqliteStoreConfig;
pub use lock::DaemonLock;

/// SQLite-backed implementation of every `ironmaint-store`
/// sub-trait.
///
/// The store holds an exclusive OS-level lock on the state
/// directory (PHASE-0B.md §10); a second `open()` against the
/// same directory returns `StoreError::Conflict`. The lock is
/// released when the store is dropped.
#[derive(Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
    config: SqliteStoreConfig,
    _lock: Arc<DaemonLock>,
}

impl SqliteStore {
    /// Open the store at `config.state_dir`, applying migrations
    /// if necessary.
    pub async fn open(config: SqliteStoreConfig) -> Result<Self, StoreError> {
        std::fs::create_dir_all(&config.state_dir).map_err(|e| {
            StoreError::new(StoreErrorKind::Backend, format!("create state dir: {e}"))
        })?;

        let lock = Arc::new(
            DaemonLock::acquire(config.state_dir.as_path()).map_err(|e| {
                StoreError::new(StoreErrorKind::Conflict, format!("daemon lock: {e}"))
            })?,
        );

        let opts = SqliteConnectOptions::new()
            .filename(config.state_dir.join("ironmaint.sqlite"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_millis(config.busy_timeout_ms));

        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect_with(opts)
            .await
            .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("open sqlite: {e}")))?;

        // PRAGMA foreign_keys is per-connection; apply on a fresh
        // connection (sqlx uses a pool so we run it via execute).
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("foreign_keys: {e}")))?;

        apply_migrations(&pool, config.migrations_dir.as_deref()).await?;

        Ok(Self {
            pool,
            config,
            _lock: lock,
        })
    }

    /// Open a test store backed by an in-memory SQLite. Used by the
    /// smoke tests. Migrations from the workspace `migrations/`
    /// directory are applied so foreign-key enforcement sees the
    /// full schema.
    #[doc(hidden)]
    pub async fn open_in_memory() -> Result<Self, StoreError> {
        use sqlx::sqlite::SqlitePoolOptions as P;
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);
        let pool = P::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("open in-mem: {e}")))?;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("foreign_keys: {e}")))?;
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("migrations"))
            .ok_or_else(|| {
                StoreError::new(
                    StoreErrorKind::Backend,
                    "in-memory: cannot locate workspace migrations dir".to_string(),
                )
            })?;
        apply_to_pool(&pool, &migrations_dir).await?;
        Ok(Self {
            pool,
            config: SqliteStoreConfig::new(":memory:"),
            _lock: Arc::new(DaemonLock::empty_for_tests()),
        })
    }

    #[must_use]
    pub fn config(&self) -> &SqliteStoreConfig {
        &self.config
    }
}

// `SqliteStore` satisfies the blanket `IronMaintStore` impl in
// `ironmaint-store` automatically (each sub-trait is implemented
// below). No explicit `impl IronMaintStore for SqliteStore` here —
// that would conflict with the blanket impl.

impl EventStore for SqliteStore {
    async fn append_event(&self, event: &ironmaint_store::EventEnvelope) -> Result<(), StoreError> {
        ops::events::append(&self.pool, event).await
    }

    async fn get_event(
        &self,
        job_id: ironmaint_core::JobId,
        sequence: u64,
    ) -> Result<ironmaint_store::EventEnvelope, StoreError> {
        ops::events::get(&self.pool, job_id, sequence).await
    }

    async fn list_events_for_job(
        &self,
        job_id: ironmaint_core::JobId,
        start: u64,
        end: Option<u64>,
    ) -> Result<Vec<ironmaint_store::EventEnvelope>, StoreError> {
        ops::events::list(&self.pool, job_id, start, end).await
    }

    async fn next_sequence(&self, job_id: ironmaint_core::JobId) -> Result<u64, StoreError> {
        ops::events::next_sequence(&self.pool, job_id).await
    }
}

impl ProjectionStore for SqliteStore {
    async fn get_projection(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<ironmaint_core::JobProjection, StoreError> {
        ops::projections::get(&self.pool, job_id).await
    }

    async fn put_projection(
        &self,
        projection: &ironmaint_core::JobProjection,
        expected_version: u64,
    ) -> Result<(), StoreError> {
        ops::projections::put(&self.pool, projection, expected_version).await
    }

    async fn rebuild_projection(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<ironmaint_core::JobProjection, StoreError> {
        ops::projections::rebuild(&self.pool, job_id).await
    }
}

impl CandidateStore for SqliteStore {
    async fn put_source_candidate(
        &self,
        candidate: &ironmaint_core::SourceCandidate,
    ) -> Result<ironmaint_core::CandidateId, StoreError> {
        ops::candidates::put_source(&self.pool, candidate).await
    }

    async fn get_source_candidate(
        &self,
        id: ironmaint_core::CandidateId,
    ) -> Result<ironmaint_core::SourceCandidate, StoreError> {
        ops::candidates::get_source(&self.pool, id).await
    }

    async fn put_release_candidate(
        &self,
        candidate: &ironmaint_policy::ReleaseCandidate,
    ) -> Result<ironmaint_core::ReleaseCandidateId, StoreError> {
        ops::candidates::put_release(&self.pool, candidate).await
    }

    async fn get_release_candidate(
        &self,
        id: ironmaint_core::ReleaseCandidateId,
    ) -> Result<ironmaint_policy::ReleaseCandidate, StoreError> {
        ops::candidates::get_release(&self.pool, id).await
    }

    async fn list_source_candidates_for_job(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Vec<ironmaint_core::CandidateId>, StoreError> {
        ops::candidates::list_source_for_job(&self.pool, job_id).await
    }

    async fn active_source_candidate(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Option<ironmaint_core::CandidateId>, StoreError> {
        ops::candidates::active_source(&self.pool, job_id).await
    }

    async fn set_active_source_candidate(
        &self,
        job_id: ironmaint_core::JobId,
        id: ironmaint_core::CandidateId,
    ) -> Result<(), StoreError> {
        ops::candidates::set_active_source(&self.pool, job_id, id).await
    }

    async fn find_source_by_fingerprint(
        &self,
        fingerprint: &ironmaint_core::CandidateFingerprint,
    ) -> Result<Option<ironmaint_core::CandidateId>, StoreError> {
        ops::candidates::find_source_by_fp(&self.pool, fingerprint).await
    }
}

impl EvidenceStore for SqliteStore {
    async fn put_evidence(
        &self,
        evidence: &ironmaint_evidence::Evidence,
        job_id: ironmaint_core::JobId,
    ) -> Result<ironmaint_core::EvidenceId, StoreError> {
        ops::evidence::put(&self.pool, evidence, job_id).await
    }

    async fn get_evidence(
        &self,
        id: ironmaint_core::EvidenceId,
    ) -> Result<ironmaint_evidence::Evidence, StoreError> {
        ops::evidence::get(&self.pool, id).await
    }

    async fn list_evidence_for_job(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Vec<ironmaint_evidence::Evidence>, StoreError> {
        ops::evidence::list_for_job(&self.pool, job_id).await
    }

    async fn list_evidence_for_candidate(
        &self,
        fingerprint: &ironmaint_core::CandidateFingerprint,
    ) -> Result<Vec<ironmaint_evidence::Evidence>, StoreError> {
        ops::evidence::list_for_candidate(&self.pool, fingerprint).await
    }
}

impl GateStore for SqliteStore {
    async fn put_gate_definition(
        &self,
        gate: &ironmaint_evidence::GateDefinition,
        job_id: ironmaint_core::JobId,
    ) -> Result<ironmaint_core::GateId, StoreError> {
        ops::gates::put_definition(&self.pool, gate, job_id).await
    }

    async fn get_gate_definition(
        &self,
        id: ironmaint_core::GateId,
    ) -> Result<ironmaint_evidence::GateDefinition, StoreError> {
        ops::gates::get_definition(&self.pool, id).await
    }

    async fn put_gate_result(
        &self,
        gate_id: ironmaint_core::GateId,
        fingerprint: &ironmaint_core::CandidateFingerprint,
        result: &ironmaint_evidence::GateResult,
    ) -> Result<(), StoreError> {
        ops::gates::put_result(&self.pool, gate_id, fingerprint, result).await
    }

    async fn get_gate_result(
        &self,
        gate_id: ironmaint_core::GateId,
        fingerprint: &ironmaint_core::CandidateFingerprint,
    ) -> Result<ironmaint_evidence::GateResult, StoreError> {
        ops::gates::get_result(&self.pool, gate_id, fingerprint).await
    }

    async fn list_gates_for_job(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Vec<ironmaint_core::GateId>, StoreError> {
        ops::gates::list_for_job(&self.pool, job_id).await
    }
}

impl ObligationStore for SqliteStore {
    async fn put_obligation(
        &self,
        obligation: &ironmaint_policy::Obligation,
        job_id: ironmaint_core::JobId,
    ) -> Result<ironmaint_core::ObligationId, StoreError> {
        ops::obligations::put(&self.pool, obligation, job_id).await
    }

    async fn get_obligation(
        &self,
        id: ironmaint_core::ObligationId,
    ) -> Result<ironmaint_policy::Obligation, StoreError> {
        ops::obligations::get(&self.pool, id).await
    }

    async fn update_obligation(
        &self,
        id: ironmaint_core::ObligationId,
        updated: &ironmaint_policy::Obligation,
    ) -> Result<(), StoreError> {
        ops::obligations::update(&self.pool, id, updated).await
    }

    async fn list_obligations_for_job(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Vec<ironmaint_core::ObligationId>, StoreError> {
        ops::obligations::list_for_job(&self.pool, job_id).await
    }
}

impl OperationStore for SqliteStore {
    async fn put_operation(
        &self,
        operation: &ironmaint_policy::PrivilegedOperation,
        job_id: ironmaint_core::JobId,
    ) -> Result<ironmaint_core::OperationId, StoreError> {
        ops::operations::put(&self.pool, operation, job_id).await
    }

    async fn get_operation(
        &self,
        id: ironmaint_core::OperationId,
    ) -> Result<ironmaint_policy::PrivilegedOperation, StoreError> {
        ops::operations::get(&self.pool, id).await
    }

    async fn update_operation(
        &self,
        id: ironmaint_core::OperationId,
        updated: &ironmaint_policy::PrivilegedOperation,
    ) -> Result<(), StoreError> {
        ops::operations::update(&self.pool, id, updated).await
    }

    async fn list_executing_operations(
        &self,
    ) -> Result<Vec<ironmaint_core::OperationId>, StoreError> {
        ops::operations::list_executing(&self.pool).await
    }

    async fn list_operations_for_job(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Vec<ironmaint_core::OperationId>, StoreError> {
        ops::operations::list_for_job(&self.pool, job_id).await
    }
}

impl ArtifactMetadataStore for SqliteStore {
    async fn put_artifact(
        &self,
        record: &ironmaint_store::artifact::ArtifactRecord,
    ) -> Result<ironmaint_core::ArtifactId, StoreError> {
        ops::artifacts::put(&self.pool, record).await
    }

    async fn get_artifact(
        &self,
        id: ironmaint_core::ArtifactId,
    ) -> Result<ironmaint_store::artifact::ArtifactRecord, StoreError> {
        ops::artifacts::get(&self.pool, id).await
    }

    async fn artifact_exists(&self, id: ironmaint_core::ArtifactId) -> Result<bool, StoreError> {
        ops::artifacts::exists(&self.pool, id).await
    }

    async fn list_artifacts_for_job(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Vec<ironmaint_core::ArtifactId>, StoreError> {
        ops::artifacts::list_for_job(&self.pool, job_id).await
    }
}

impl WorkspaceMetadataStore for SqliteStore {
    async fn get_workspace_state(
        &self,
        handle: &ironmaint_store::workspace::WorkspaceHandle,
    ) -> Result<ironmaint_store::workspace::WorkspaceState, StoreError> {
        ops::workspaces::get(&self.pool, handle).await
    }

    async fn put_workspace_state(
        &self,
        state: &ironmaint_store::workspace::WorkspaceState,
        expected_revision: u64,
    ) -> Result<(), StoreError> {
        ops::workspaces::put(&self.pool, state, expected_revision).await
    }

    async fn list_workspaces_for_job(
        &self,
        job_id: ironmaint_core::JobId,
    ) -> Result<Vec<ironmaint_store::workspace::WorkspaceHandle>, StoreError> {
        ops::workspaces::list_for_job(&self.pool, job_id).await
    }
}

/// Apply any committed migrations that have not been applied yet.
async fn apply_migrations(
    pool: &SqlitePool,
    migrations_dir: Option<&Path>,
) -> Result<(), StoreError> {
    let Some(dir) = migrations_dir else {
        return Ok(());
    };
    let files = collect_migration_files(dir)?;
    if files.is_empty() {
        return Ok(());
    }

    pool.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             filename   TEXT    NOT NULL UNIQUE,
             applied_at TEXT    NOT NULL
         )",
    )
    .await
    .map_err(|e| {
        StoreError::new(
            StoreErrorKind::Backend,
            format!("create schema_migrations: {e}"),
        )
    })?;

    for (version, filename, sql) in files {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT version FROM schema_migrations WHERE filename = ?1")
                .bind(filename.as_str())
                .fetch_optional(pool)
                .await
                .map_err(|e| {
                    StoreError::new(
                        StoreErrorKind::Backend,
                        format!("query schema_migrations: {e}"),
                    )
                })?;
        if row.is_some() {
            continue;
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("begin tx: {e}")))?;
        // `raw_sql` in sqlx 0.9 requires `&'static str`. The
        // migration strings are loaded once at startup; leaking
        // them for `'static` is intentional.
        let sql_static: &'static str = Box::leak(sql.into_boxed_str());
        sqlx::raw_sql(sql_static)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                StoreError::new(StoreErrorKind::Backend, format!("apply {filename}: {e}"))
            })?;
        let applied_at = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| {
                StoreError::new(StoreErrorKind::Backend, format!("format rfc3339: {e}"))
            })?;
        sqlx::query(
            "INSERT INTO schema_migrations (version, filename, applied_at) VALUES (?1, ?2, ?3)",
        )
        .bind(version)
        .bind(filename.as_str())
        .bind(applied_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("record migration: {e}")))?;
        tx.commit().await.map_err(|e| {
            StoreError::new(StoreErrorKind::Backend, format!("commit migration: {e}"))
        })?;
    }
    Ok(())
}

/// Apply migrations against an in-memory SQLite pool, returning
/// the set of created tables. Used by `verify-migrations` to build
/// a snapshot of the expected schema.
pub async fn apply_to_pool(pool: &SqlitePool, dir: &Path) -> Result<(), StoreError> {
    let files = collect_migration_files(dir)?;
    for (_version, _filename, sql) in files {
        let sql_static: &'static str = Box::leak(sql.into_boxed_str());
        sqlx::raw_sql(sql_static).execute(pool).await.map_err(|e| {
            StoreError::new(StoreErrorKind::Backend, format!("apply migration: {e}"))
        })?;
    }
    Ok(())
}

/// Collect `(version, filename, sql)` triples from the migrations
/// directory. Returns them sorted by version.
fn collect_migration_files(dir: &Path) -> Result<Vec<(i64, String, String)>, StoreError> {
    let mut out: Vec<(i64, String, String)> = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| {
        StoreError::new(
            StoreErrorKind::Backend,
            format!("read migrations dir {}: {e}", dir.display()),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            StoreError::new(
                StoreErrorKind::Backend,
                format!("read migrations entry: {e}"),
            )
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".sql") {
            continue;
        }
        let version: i64 = name
            .split('_')
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                StoreError::new(
                    StoreErrorKind::Backend,
                    format!("migration filename {name} does not start with a version number"),
                )
            })?;
        let sql = std::fs::read_to_string(&path).map_err(|e| {
            StoreError::new(
                StoreErrorKind::Backend,
                format!("read migration {name}: {e}"),
            )
        })?;
        out.push((version, name.to_owned(), sql));
    }
    out.sort_by_key(|(v, _, _)| *v);
    Ok(out)
}

/// Path of the SQLite database file inside a state directory.
#[must_use]
pub fn db_path(state_dir: &Path) -> PathBuf {
    state_dir.join("ironmaint.sqlite")
}
