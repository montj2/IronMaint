//! PHASE-1.1 PR 1.1.F — both-stores conformance pin for the
//! candidate-job referential integrity (D-21).
//!
//! `store_parity_sqlite.rs` already pins the orphan-candidate
//! rejection at the SQLite layer (the `FOREIGN KEY` constraint on
//! `source_candidates.job_id` is enforced via
//! `PRAGMA foreign_keys = ON`). The pre-1.1.F `MockStore` did not
//! enforce the same invariant; tests of the `candidate.capture`
//! path that ran on the mock silently succeeded even when the
//! job did not exist, while the production path through SQLite
//! returned a raw `FOREIGN KEY constraint failed`. The
//! `a_candidate_for_an_unknown_job_is_refused` case in
//! `store_parity_sqlite.rs` is the architectural shape of the
//! regression the D-21 closure was tracking.
//!
//! PR 1.1.F closes D-21 at the store layer by:
//! 1. Adding a `put_source_candidate` candidate-job check in
//!    `MockStore` that mirrors the SQLite `FOREIGN KEY`.
//! 2. Adding this both-stores conformance case, which walks the
//!    same assertion against both backends. The existing
//!    `store_parity_sqlite.rs` case stays as a belt-and-braces
//!    pin at the SQLite layer.
//!
//! ## RED state
//!
//! The pre-1.1.F `MockStore` accepts orphan candidates (no
//! candidate-job referential integrity check), so this test
//! fails against `MockStore::default()` even though it passes
//! against `SqliteStore::open(...)`. The GREEN state adds the
//! missing check to `MockStore::put_source_candidate`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate,
    VcsKind,
};
use ironmaint_store::{CandidateStore, StoreErrorKind};
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use time::macros::datetime;
use url::Url;

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");

async fn migrated_sqlite(name: &str) -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let config = SqliteStoreConfig::new(dir.path().join(name)).with_migrations_dir(MIGRATIONS_DIR);
    let store = SqliteStore::open(config)
        .await
        .expect("open the migrated store");
    (dir, store)
}

/// Walk the candidate-job FK conformance case against both
/// backends. The shape of the case is the same one
/// `store_parity_sqlite.rs::a_candidate_for_an_unknown_job_is_refused`
/// uses for the SQLite-only pin; here the helper is invoked
/// against `MockStore::default()` and `SqliteStore::open(...)`
/// so a failure names the backend.
///
/// `MockStore` is the pre-1.1.F failure case: the orphan
/// candidate is accepted because `MockStore::put_source_candidate`
/// has no candidate-job referential integrity check. SQLite
/// has always rejected the orphan (the
/// `source_candidates.job_id` `FOREIGN KEY` is in
/// `migrations/0001_initial.sql` and `PRAGMA foreign_keys = ON`
/// is set on every connection).
#[tokio::test]
async fn a_candidate_for_an_unknown_job_is_refused_by_both_stores() {
    let mock = ironmaint_store::mock::MockStore::new();
    assert_candidate_job_fk(&mock, "MockStore").await;

    let (_dir, sqlite) = migrated_sqlite("orphan-both-stores.sqlite").await;
    assert_candidate_job_fk(&sqlite, "SqliteStore").await;
}

async fn assert_candidate_job_fk<S>(store: &S, backend_name: &str)
where
    S: CandidateStore,
{
    let ghost_job = JobId::new();
    let error = store
        .put_source_candidate(&orphan_candidate(ghost_job))
        .await
        .expect_err(&format!(
            "{backend_name}: a candidate hanging off a job that was never created must not store"
        ));

    assert!(
        matches!(
            error.kind,
            StoreErrorKind::Backend | StoreErrorKind::Conflict
        ),
        "{backend_name}: expected a refusal (Backend or Conflict), got {:?} ({})",
        error.kind,
        error.detail
    );
    assert!(
        store
            .list_source_candidates_for_job(ghost_job)
            .await
            .expect("list")
            .is_empty(),
        "{backend_name}: the refused write must leave no row behind"
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
