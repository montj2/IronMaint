//! §36 regression tests: `rebuild-projections` must actually rebuild —
//! and must refuse when it cannot.
//!
//! The tool an operator reaches for when a projection row has drifted
//! from the event log could not succeed on **any** real database. It
//! took its CAS token from the *rebuilt* projection's version, and
//! `rebuild_projection` starts from the `JobCreated` seed rather than
//! reading the row it is repairing — so the token was 0 for any job
//! that had ever advanced, and `put_projection` rejected it with
//! `Conflict: expected_version=0, found=1`.
//!
//! Fixing only that turned a loud failure into silent data loss, which
//! is the subject of the second test.
//!
//! The shape of the miss is D-14's again, and worth naming: the
//! pre-existing `rebuild_smoke.rs` exercises the **no-events** path —
//! the one path that always worked — and asserts only that the CLI
//! prints a report. Nothing drove a real job through `CreateJob` and
//! then asked for a rebuild.
//!
//! So that is what these do: the real command surface produces the
//! log, against a real SQLite store, and the assertions are on what
//! the tool reports and what the store says afterwards.
//!
//! Found by hand, running the real binary against a live daemon's
//! state directory after 0B.10's tool surface was in place. No test
//! could have found it, for the reason in the paragraph above.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    JobState, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
    SourceCandidate, VcsKind,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{
    OrchestratorRef, RuntimeCommand, RuntimeService,
    clock::{Clock, FixedClock},
};
use ironmaint_store::ProjectionStore;
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use time::OffsetDateTime;

/// Path to the workspace `migrations/` directory. Cargo does not put
/// it on the test's path, and `SqliteStore::open` applies migrations
/// only when told where they are.
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");

fn store_config(dir: &std::path::Path) -> SqliteStoreConfig {
    SqliteStoreConfig::new(dir).with_migrations_dir(std::path::Path::new(MIGRATIONS_DIR))
}

fn service(store: Arc<SqliteStore>) -> RuntimeService<SqliteStore, NullExecutor> {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    ));
    RuntimeService::new(
        store,
        clock,
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

/// Drive the real `CreateJob` path, so the log is what the runtime
/// actually writes rather than something this test assembles.
async fn create_real_job(store: Arc<SqliteStore>) -> JobId {
    let result = service(store)
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("CreateJob must succeed against a real store");

    // `CommandResult` carries no id; the minted `JobId` is named in
    // the side effect, which is how every caller reads it.
    result.side_effects[0]
        .strip_prefix("job:")
        .expect("the side effect names the job")
        .split_whitespace()
        .next()
        .expect("and the id follows the prefix")
        .parse()
        .expect("the id parses")
}

/// Then the real `CaptureCandidate` path, which is what activates the
/// candidate — and activation is the thing the log cannot replay, so
/// this is the only way to reach the state the second test needs.
///
/// It also bumps the stored projection to version 1, which is why
/// **a job that never advances is a job the CAS bug is invisible
/// on**: the replay's version and the stored version are both 0, so
/// the wrong token accidentally matches. The first draft of the CAS
/// test stopped at `CreateJob` and passed against the broken code.
async fn capture_on(store: Arc<SqliteStore>, job_id: JobId) {
    service(store)
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id,
            candidate: source_candidate(job_id),
        })
        .await
        .expect("CaptureCandidate must succeed against a real store");
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("sid").unwrap(),
        ),
        PackageName::new("rebuild-regression").unwrap(),
    )
}

fn source_candidate(job_id: JobId) -> SourceCandidate {
    let url = url::Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap();
    let revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap());
    SourceCandidate::new(
        job_id,
        revision,
        repository,
        commit,
        tree,
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    )
}

/// A row whose `version` does not match what the log implies, and
/// whose `state` the log does not support.
///
/// This is drift, written through the store's own API rather than by
/// editing SQLite — which is the point. §36 exists for exactly this
/// input, and the tool is the only thing that can produce it, so a
/// hand-arranged row is a *legitimate* fixture where a hand-assembled
/// log is not (D-14).
///
/// A version bump is included alongside the state corruption because
/// a drifted version is the other half of what makes the CAS token
/// wrong, and it is the half that cannot be reached through any
/// production path: every real version bump goes through
/// `activate_candidate`, which is exactly what the second test is
/// about.
async fn corrupt_row(store: &SqliteStore, job_id: JobId, version: u64) {
    let mut drifted = store
        .get_projection(job_id)
        .await
        .expect("CreateJob seeds the projection");
    drifted.state = JobState::ReadyForApproval;
    drifted.version = version;
    // `put_projection` writes `projection.version`, and the token is
    // the version the row currently holds — 0, straight from
    // `CreateJob`.
    store
        .put_projection(&drifted, 0)
        .await
        .expect("drift the row");
}

#[tokio::test]
async fn rebuilding_a_drifted_row_repairs_it_without_rewinding_the_version() {
    let tmp = tempfile::tempdir().unwrap();
    let job_id;
    let before_source;

    {
        let store = Arc::new(
            SqliteStore::open(store_config(tmp.path()))
                .await
                .expect("open store"),
        );
        job_id = create_real_job(Arc::clone(&store)).await;
        before_source = store
            .get_projection(job_id)
            .await
            .expect("CreateJob seeds the projection")
            .job
            .package
            .source_name
            .clone();

        corrupt_row(&store, job_id, 5).await;

        // The bug's precondition, asserted directly: the replayed
        // projection's version is whatever the log implies, which is
        // 0 for a job whose only event is its birth — whatever is
        // stored. That is why the CAS token cannot come from it.
        assert_eq!(
            store
                .rebuild_projection(job_id)
                .await
                .expect("the log replays")
                .version,
            0,
            "the replay's version is 0 whatever is stored — which is the \
             whole reason the CAS token cannot come from it"
        );
    }

    // `run` opens its own handle, exactly as the binary does.
    let report = ironmaintctl::rebuild::run(tmp.path(), Some(job_id))
        .await
        .expect("rebuild-projections must not fail to open the store");

    assert!(
        report.errors.is_empty(),
        "the §36 escape hatch must rebuild a real drifted row, got: {:?}",
        report.errors
    );
    assert_eq!(
        report.rebuilt,
        vec![job_id],
        "one job requested, one job rebuilt"
    );
    assert!(report.skipped.is_empty());

    let store = SqliteStore::open(store_config(tmp.path()))
        .await
        .expect("reopen store");
    let after = store
        .get_projection(job_id)
        .await
        .expect("projection after rebuild");

    assert_eq!(
        after.state,
        JobState::EventDetected,
        "the rebuild restores the state the log justifies, discarding the \
         corruption the row had drifted into"
    );
    // A rebuild repairs the row; it must not rewind the version.
    // `put_projection` writes `projection.version`, so passing the
    // right CAS token while carrying the replayed one would succeed
    // and quietly disarm the optimistic-concurrency check for
    // whatever wrote last.
    assert_eq!(
        after.version, 5,
        "the stored version is carried forward, not rewound to the log's 0"
    );
    assert_eq!(
        after.job.package.source_name, before_source,
        "the package survives the round trip through the log"
    );
}

#[tokio::test]
async fn a_rebuild_refuses_rather_than_dropping_the_active_candidate() {
    let tmp = tempfile::tempdir().unwrap();
    let job_id;
    let before;

    {
        let store = Arc::new(
            SqliteStore::open(store_config(tmp.path()))
                .await
                .expect("open store"),
        );
        job_id = create_real_job(Arc::clone(&store)).await;
        capture_on(Arc::clone(&store), job_id).await;
        before = store
            .get_projection(job_id)
            .await
            .expect("the capture seeded the projection");

        // The precondition, asserted rather than assumed. Without
        // this, the test would pass against a build whose refusal
        // never fired — for the same reason the first draft of the
        // CAS test passed against the broken code.
        assert!(
            before.active_candidate.is_some(),
            "capture must have activated a candidate, or this test proves nothing"
        );
    }

    let report = ironmaintctl::rebuild::run(tmp.path(), Some(job_id))
        .await
        .expect("rebuild-projections must not fail to open the store");

    assert!(
        report.rebuilt.is_empty(),
        "a job whose log cannot justify the active candidate must not be \
         reported as rebuilt, got: {:?}",
        report.rebuilt
    );
    assert_eq!(
        report.errors.len(),
        1,
        "one job requested, one named refusal, got: {:?}",
        report.errors
    );
    // The refusal has to be *nameable*. "rebuild failed" tells an
    // operator nothing they can act on; the whole value of refusing
    // is that the message says which fact the log is missing.
    assert!(
        report.errors[0].contains("does not record candidate activation"),
        "the refusal must name the missing event, got: {}",
        report.errors[0]
    );

    let store = SqliteStore::open(store_config(tmp.path()))
        .await
        .expect("reopen store");
    let after = store
        .get_projection(job_id)
        .await
        .expect("projection after the refused rebuild");

    // The point of the whole exercise. §30 binds every gate verdict to
    // the active candidate, so a rebuild that "succeeds" by dropping
    // it does not degrade the row — it invalidates the evidence for
    // every check already run against it.
    assert_eq!(
        after.active_candidate, before.active_candidate,
        "a refused rebuild must leave the active candidate exactly as it was"
    );
    assert_eq!(after.version, before.version, "and the version too");
    assert_eq!(after.state, before.state, "and the state too");
}
