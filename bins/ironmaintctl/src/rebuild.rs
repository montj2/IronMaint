//! `rebuild-projections` subcommand.
//!
//! Walks the SQLite event log and reconstructs the projection for
//! one job (or, when a future commit adds a `list_jobs` facade,
//! every job). Replays each event through
//! `ProjectionStore::rebuild_projection`, then writes the rebuilt
//! projection back via `put_projection` using the existing version
//! as the CAS token.
//!
//! PHASE-0B.md §36: this is the operator escape hatch when a
//! projection row drifts from the event log (manual edits, recovery
//! from a corrupted DB, etc.).

use std::fmt;
use std::path::Path;

use ironmaint_core::JobId;
use ironmaint_store::{EventStore, ProjectionStore};
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};

pub struct RebuildReport {
    pub rebuilt: Vec<JobId>,
    pub skipped: Vec<JobId>,
    pub errors: Vec<String>,
}

impl fmt::Display for RebuildReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "rebuild-projections\n  rebuilt: {} job(s)\n  skipped: {} job(s)",
            self.rebuilt.len(),
            self.skipped.len()
        )?;
        if !self.errors.is_empty() {
            writeln!(f, "  errors:")?;
            for e in &self.errors {
                writeln!(f, "    - {e}")?;
            }
        }
        Ok(())
    }
}

pub async fn run(state_dir: &Path, only_job: Option<JobId>) -> Result<RebuildReport, String> {
    let cfg = SqliteStoreConfig::new(state_dir);
    let store = SqliteStore::open(cfg)
        .await
        .map_err(|e| format!("open store: {e}"))?;

    let targets: Vec<JobId> = match only_job {
        Some(j) => vec![j],
        None => {
            // Phase 0B does not yet expose a `list_jobs` method on
            // the store facade (that lands once the runtime's
            // command surface is wired). Operator must pass
            // `--job <id>` until that path is available.
            eprintln!("note: rebuilding every job requires a future facade; pass `--job <id>`.");
            Vec::new()
        }
    };

    let mut report = RebuildReport {
        rebuilt: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
    };

    for jid in targets {
        match replay_one(&store, jid).await {
            Ok(true) => report.rebuilt.push(jid),
            Ok(false) => report.skipped.push(jid),
            Err(e) => report.errors.push(format!("{jid}: {e}")),
        }
    }
    Ok(report)
}

async fn replay_one(store: &SqliteStore, jid: JobId) -> Result<bool, String> {
    let events = store
        .list_events_for_job(jid, 1, None)
        .await
        .map_err(|e| format!("list events: {e}"))?;
    if events.is_empty() {
        return Ok(false);
    }
    let proj = store
        .rebuild_projection(jid)
        .await
        .map_err(|e| format!("rebuild: {e}"))?;
    let expected = proj.version;
    store
        .put_projection(&proj, expected)
        .await
        .map_err(|e| format!("put projection: {e}"))?;
    Ok(true)
}
