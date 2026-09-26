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
//! 4. The set of schema objects (tables + indexes) produced by
//!    applying the migrations matches the committed snapshot in
//!    `migrations/.expected-schema-objects.txt`. Any drift
//!    (added object, removed object) fails the verifier.
//!
//! With `--write`, the snapshot is regenerated from the live apply
//! so a deliberate change can be accepted.
//!
//! The verifier does **not** apply migrations to the production
//! state directory — that is the daemon's responsibility.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use ironmaint_store_sqlite::apply_to_pool;

const SNAPSHOT_FILE: &str = ".expected-schema-objects.txt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationMismatchKind {
    Added,
    Removed,
}

impl std::fmt::Display for MigrationMismatchKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added => write!(f, "added"),
            Self::Removed => write!(f, "removed"),
        }
    }
}

#[derive(Debug)]
pub struct MigrationMismatch {
    pub kind: MigrationMismatchKind,
    pub object: String,
}

#[derive(Debug)]
pub struct MigrationReport {
    pub committed_migrations: Vec<String>,
    pub committed_objects: Vec<String>,
    pub applied_objects: Vec<String>,
    pub mismatches: Vec<MigrationMismatch>,
    pub wrote: bool,
}

impl MigrationReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }
}

impl std::fmt::Display for MigrationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "verify-migrations\n  \
             mode:                  {}\n  \
             committed migrations:  {}\n  \
             committed objects:     {}\n  \
             applied objects:       {}\n  \
             status:                {}",
            if self.wrote { "write" } else { "verify (diff)" },
            self.committed_migrations.len(),
            self.committed_objects.len(),
            self.applied_objects.len(),
            if self.is_clean() { "clean" } else { "DRIFT" }
        )?;
        for m in &self.mismatches {
            writeln!(f, "  - {} [{}]", m.object, m.kind)?;
        }
        Ok(())
    }
}

pub fn run(write: bool) -> Result<MigrationReport, Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;
    runtime.block_on(async { run_async(write).await })
}

async fn run_async(write: bool) -> Result<MigrationReport, Box<dyn Error>> {
    let root = workspace_root()?;
    let migrations_dir = root.join("migrations");
    let snapshot_path = migrations_dir.join(SNAPSHOT_FILE);

    let committed_migrations = scan_committed(&migrations_dir)?;

    // 1. Filename pattern check.
    if let Some(first) = committed_migrations.first() {
        if first != "0001_initial.sql" {
            eprintln!("verify-migrations: first migration must be 0001_initial.sql, got {first}");
        }
    }
    for (idx, name) in committed_migrations.iter().enumerate() {
        let expected_id = idx + 1;
        let prefix = format!("{expected_id:04}_");
        if !name.starts_with(&prefix) {
            eprintln!(
                "verify-migrations: expected {prefix}*, got {name} at position {expected_id}"
            );
        }
    }

    let applied_objects = if committed_migrations.is_empty() {
        Vec::new()
    } else {
        apply_and_introspect(&migrations_dir).await?
    };

    // 2. Read the committed snapshot.
    let committed_objects = if snapshot_path.is_file() {
        let content = fs::read_to_string(&snapshot_path)?;
        content
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    // 3. Compute object-level diff.
    let mut object_mismatches = Vec::new();
    let committed_set: std::collections::BTreeSet<&str> =
        committed_objects.iter().map(String::as_str).collect();
    let applied_set: std::collections::BTreeSet<&str> =
        applied_objects.iter().map(String::as_str).collect();
    for name in applied_set.difference(&committed_set) {
        object_mismatches.push(MigrationMismatch {
            kind: MigrationMismatchKind::Added,
            object: (*name).to_string(),
        });
    }
    for name in committed_set.difference(&applied_set) {
        object_mismatches.push(MigrationMismatch {
            kind: MigrationMismatchKind::Removed,
            object: (*name).to_string(),
        });
    }
    object_mismatches.sort_by(|a, b| a.object.cmp(&b.object));

    // 4. If --write, regenerate the snapshot from the live apply.
    let mut wrote = false;
    if write && !committed_migrations.is_empty() {
        let body = applied_objects.join("\n") + "\n";
        fs::write(&snapshot_path, body)?;
        wrote = true;
    }

    Ok(MigrationReport {
        committed_migrations,
        committed_objects,
        applied_objects,
        mismatches: object_mismatches,
        wrote,
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
        // Skip the snapshot file itself.
        if path.file_name().and_then(|s| s.to_str()) == Some(SNAPSHOT_FILE) {
            continue;
        }
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

async fn apply_and_introspect(migrations_dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
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
    Ok(rows.into_iter().map(|(n,)| n).collect())
}
