//! S9 driver — [`SqliteStore`] against the store-conformance script.
//!
//! Half of the pair. The other half is `store_parity.rs`, and the two
//! files are the same test in the same shape against the two backends.
//! Split rather than combined so that a failure names the backend.
//!
//! ## Why this opens a real file rather than `open_in_memory`
//!
//! `SqliteStore::open_in_memory` **skips migrations**, so it hands back
//! a pool whose tables are whatever the pool's own bootstrap created —
//! not the schema in `migrations/`. `source_candidates.fingerprint` is
//! `UNIQUE` only in the real migration, so the duplicate-fingerprint
//! case would pass here for entirely the wrong reason: the constraint
//! the case is about would not exist.
//!
//! That is the same shape as the defect this suite exists to catch — a
//! green test over a hole — one level down, and it is worth naming
//! rather than leaving for the next reader to rediscover. This driver
//! therefore opens a real database with migrations applied, exactly as
//! the §101 acceptance driver does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate,
    VcsKind,
};
use ironmaint_store::{CandidateStore, StoreErrorKind};
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use ironmaint_testkit::assert_store_conformance;
use time::macros::datetime;
use url::Url;

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");

async fn migrated_store(name: &str) -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let config = SqliteStoreConfig::new(dir.path().join(name)).with_migrations_dir(MIGRATIONS_DIR);
    let store = SqliteStore::open(config)
        .await
        .expect("open the migrated store");
    (dir, store)
}

#[tokio::test]
async fn sqlite_store_satisfies_the_store_contract() {
    let (_dir, store) = migrated_store("store-parity.sqlite").await;
    assert_store_conformance(&store).await;
}

/// A candidate whose job does not exist is refused.
///
/// **This is the second divergence the S9 script found, and the more
/// serious of the two, because it is reachable from the tool surface.**
///
/// `source_candidates.job_id` is a `FOREIGN KEY` to `jobs(id)` and
/// `SqliteStore` sets `PRAGMA foreign_keys = ON` on every connection
/// (`crates/ironmaint-store-sqlite/src/lib.rs:90,121`), so the
/// constraint is genuinely enforced. `MockStore` has no referential
/// integrity and accepts the orphan.
///
/// Nothing between the agent and the insert creates the job. The
/// `job.capture` dispatcher (`crates/ironmaint-mcp/src/dispatch.rs:263`)
/// takes `input.job_id` verbatim, `ensure_workspace` does not create a
/// job, and `handle_capture_candidate`
/// (`crates/ironmaint-runtime/src/service.rs:1461`) checks only that
/// `candidate.job_id() == job_id` before writing. So an agent that
/// calls `job.capture` for a job it has not created gets a raw
/// `FOREIGN KEY constraint failed` out of SQLite in production, while
/// every test of that path — all of which run on the mock — sees it
/// succeed.
///
/// The case lives here rather than in the shared script because the
/// mock cannot pass it. Putting it in the script would mean either a
/// permanently red driver or a mock behaviour change with a measured
/// blast radius of 7 tests across 5 binaries, most of them in
/// `ironmaint-workspace`'s capture tests, which build a candidate for a
/// job that was never created — so the disagreement underneath this is
/// real and is a design question about who creates the job first, not a
/// missing guard. Recorded as D-21; see the module doc of
/// `store_conformance.rs` for the same divergence from the other side.
#[tokio::test]
async fn a_candidate_for_an_unknown_job_is_refused() {
    let (_dir, store) = migrated_store("orphan.sqlite").await;

    let ghost_job = JobId::new();
    let error = store
        .put_source_candidate(&orphan_candidate(ghost_job))
        .await
        .expect_err("a candidate hanging off a job that was never created must not store");

    assert!(
        matches!(
            error.kind,
            StoreErrorKind::Backend | StoreErrorKind::Conflict
        ),
        "expected a refusal, got {:?} ({})",
        error.kind,
        error.detail
    );
    assert!(
        store
            .list_source_candidates_for_job(ghost_job)
            .await
            .expect("list")
            .is_empty(),
        "the refused write must leave no row behind"
    );
}

fn orphan_candidate(job: JobId) -> SourceCandidate {
    let at = datetime!(2026-01-01 00:00:00 UTC);
    SourceCandidate::new(
        job,
        PackageRevision::new(
            PackageIdentity::new(
                DistributionRef::new(
                    DistributionFamily::new("conformance").expect("a valid family"),
                    DistributionRelease::new("conformance").expect("a valid release"),
                ),
                PackageName::new("store-conformance").expect("a valid name"),
            ),
            PackageVersion::new("1.0.0").expect("a valid version"),
        ),
        RepositoryRef::new(
            VcsKind::Git,
            Url::parse("https://example.invalid/orphan.git").expect("static URL parses"),
        )
        .expect("a valid repository reference"),
        GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).expect("a valid object id"),
        GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).expect("a valid object id"),
        at,
    )
}
