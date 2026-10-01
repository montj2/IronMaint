//! `rebuild-projections` subcommand.
//!
//! Walks the SQLite event log and reconstructs the projection for
//! one job (or, when a future commit adds a `list_jobs` facade,
//! every job). Replays each event through
//! `ProjectionStore::rebuild_projection`, then writes the rebuilt
//! projection back via `put_projection` using the version **stored
//! on the row being repaired** as the CAS token, and carrying that
//! same version on the write.
//!
//! PHASE-0B.md §36: this is the operator escape hatch when a
//! projection row drifts from the event log (manual edits, recovery
//! from a corrupted DB, etc.).

use std::fmt;
use std::path::Path;

use ironmaint_core::JobId;
use ironmaint_store::{EventStore, ProjectionStore, StoreErrorKind};
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
    let mut proj = store
        .rebuild_projection(jid)
        .await
        .map_err(|e| format!("rebuild: {e}"))?;

    // The CAS token is the version **currently stored**, not the one
    // the replay happened to arrive at. `rebuild_projection` starts
    // from the `JobCreated` seed and applies events; it does not read
    // the row it is meant to repair, so its `version` is whatever the
    // log implies — 0 for any job whose only event is its birth.
    //
    // Taking `proj.version` as the token meant `put_projection`
    // compared 0 against the real stored 1 and returned
    // `Conflict: expected_version=0, found=1` for **every job that had
    // ever advanced**. The tool an operator reaches for when a
    // projection has already drifted could therefore never succeed,
    // on any real database.
    let stored = match store.get_projection(jid).await {
        Ok(stored) => Some(stored),
        // No row to repair yet. `put_projection` treats
        // `expected_version = 0` as "this is the first write", which
        // is exactly the case.
        Err(e) if *e.kind() == StoreErrorKind::NotFound => None,
        Err(e) => return Err(format!("read stored projection: {e}")),
    };
    let expected = stored.as_ref().map_or(0, |s| s.version);

    // **Refuse rather than clobber: `active_candidate` is not in the
    // log.** `activate_candidate` appends `JobEvent::Domain(uuid)` —
    // a bare identifier with no payload — so a replay reproduces the
    // job's `state` but always arrives with `active_candidate: None`.
    //
    // Fixing only the CAS token above turns the old loud failure into
    // silent data loss: the write would succeed, report `rebuilt: 1`,
    // and drop the active-candidate binding that §30 makes every
    // gate verdict depend on. That is worse in every way a repair
    // tool can be worse.
    //
    // So this tool rebuilds what the log can justify and refuses, by
    // name, what it cannot. Making the log able to justify it means
    // giving activation a real event variant, which changes the
    // durable event format and is its own piece of work — see
    // `doc/DEBT.md` D-16.
    if let Some(stored) = &stored
        && stored.active_candidate != proj.active_candidate
    {
        return Err(format!(
            "refusing to rebuild: the event log does not record candidate \
             activation, so the replay would drop the active candidate \
             (stored {:?}, replayed {:?}). Repair this row by hand, or \
             wait for the log to carry activation.",
            stored.active_candidate, proj.active_candidate,
        ));
    }

    // The rebuilt projection must carry the stored version:
    // `put_projection` writes `projection.version`, so passing the
    // right token and writing the wrong one would roll the version
    // backwards and disarm the optimistic-concurrency check for
    // whatever wrote it last.
    proj.version = expected;
    store
        .put_projection(&proj, expected)
        .await
        .map_err(|e| format!("put projection: {e}"))?;
    Ok(true)
}
