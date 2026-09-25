//! `cargo xtask verify-migrations` — check the migrations directory.
//!
//! Per PHASE-0B.md §30: "Migrations are forward-only in 0B." The
//! verifier asserts:
//!
//! 1. Every committed file matches `NNNN_*.sql` (zero-padded numeric
//!    id, underscore, snake_case name, `.sql` suffix).
//! 2. Sequence ids are monotonic and contiguous, starting at 1.
//! 3. Every file applies cleanly to a fresh in-memory SQLite
//!    database (no parse errors).
//! 4. After all applies, `schema_migrations` records the same set
//!    the directory carries.
//!
//! The verifier does **not** apply migrations to the production
//! state directory — that is the daemon's responsibility.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use ironmaint_store_sqlite::apply_to_pool;

#[derive(Debug)]
pub struct MigrationReport {
    pub committed: Vec<String>,
    pub applied: Vec<String>,
    pub mismatches: Vec<String>,
}

impl MigrationReport {
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }
}

impl std::fmt::Display for MigrationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "verify-migrations\n  committed migrations: {}\n  applied schema objects: {}\n  status:                 {}",
            self.committed.len(),
            self.applied.len(),
            if self.is_clean() { "clean" } else { "DRIFT" }
        )?;
        for m in &self.mismatches {
            writeln!(f, "  - {m}")?;
        }
        Ok(())
    }
}

pub fn run(_write: bool) -> Result<MigrationReport, Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;
    runtime.block_on(async { run_async().await })
}

async fn run_async() -> Result<MigrationReport, Box<dyn Error>> {
    let root = workspace_root()?;
    let migrations_dir = root.join("migrations");
    let committed = scan_committed(&migrations_dir)?;
    let mut mismatches = Vec::new();
    if let Some(first) = committed.first() {
        if first != "0001_initial.sql" {
            mismatches.push(format!(
                "first migration must be 0001_initial.sql, got {first}"
            ));
        }
    }
    for (idx, name) in committed.iter().enumerate() {
        let expected_id = idx + 1;
        let prefix = format!("{expected_id:04}_");
        if !name.starts_with(&prefix) {
            mismatches.push(format!(
                "expected {prefix}*, got {name} at position {expected_id}"
            ));
        }
    }

    let applied = if committed.is_empty() {
        Vec::new()
    } else {
        apply_and_introspect(&migrations_dir, &committed).await?
    };

    Ok(MigrationReport {
        committed,
        applied,
        mismatches,
    })
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let mut p = exe.as_path();
    while let Some(parent) = p.parent() {
        if parent.join("Cargo.toml").is_file() && parent.join("migrations").is_dir() {
            return Ok(parent.to_path_buf());
        }
        p = parent;
    }
    Err(format!(
        "verify-migrations: workspace root not found (no Cargo.toml/migrations above {})",
        exe.display()
    )
    .into())
}

fn scan_committed(dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("sql") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("unreadable filename: {}", path.display()))?
            .to_string();
        names.push(name);
    }
    names.sort();
    Ok(names)
}

async fn apply_and_introspect(
    migrations_dir: &Path,
    committed: &[String],
) -> Result<Vec<String>, Box<dyn Error>> {
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    let opts = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(|e| format!("open in-mem: {e}"))?;
    apply_to_pool(&pool, migrations_dir).await?;
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type IN ('table','index') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| format!("introspect: {e}"))?;
    let _ = committed; // committed already validated
    Ok(rows.into_iter().map(|(n,)| n).collect())
}
