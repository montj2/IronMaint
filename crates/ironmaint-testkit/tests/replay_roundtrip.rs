//! Seam check S7 — the projection must be reconstructible from the
//! event log alone.
//!
//! ## What this asserts, and why it is shaped this way
//!
//! `JobProjection` has five fields. Fourteen defects during 0B.10
//! were all the same shape: a component modelled something correctly
//! and no production path produced the input. This file is the
//! general form of that check for the one place it is measurable —
//! the log is the system's authority (§36), and `rebuild-projections`
//! is the operator's tool for trusting it.
//!
//! The assertion is field-by-field, driven by **production commands
//! against a real SQLite store**, and never by a hand-assembled log.
//! That last clause is the whole design. Every pre-existing
//! `rebuild_projection` test hand-built its log (D-14), which is why
//! a defect affecting 100% of real jobs passed CI. D-14's fix made
//! the log authoritative for a job's *birth*; it did not make it
//! authoritative for anything written after it (D-16), and the two
//! `rebuild_projection` seed decisions that had drifted into separate
//! backends are the reason this file drives commands rather than
//! events.
//!
//! The failure message names **which fields** the log cannot justify,
//! because "replay differs from stored" is not an actionable defect
//! report and a repair tool that cannot say which fact it is missing
//! is the §36 problem restated.
//!
//! ## Location
//!
//! `doc/SEAM-VERIFICATION.md` §3 puts S7 in
//! `ironmaint-store-sqlite/tests/`. That is not possible: this test
//! needs `RuntimeService` to produce the projection, and
//! `ironmaint-runtime` depends on the store, not the reverse. The
//! store crate cannot reach the only thing worth asserting against.
//! It lives in the testkit, which §98 already blesses for a
//! store-backend dev-dependency (C4's scenario needed a real
//! restartable store for the same reason).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    JobProjection, JobState, PackageIdentity, PackageName, PackageRevision, PackageVersion,
    RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{OrchestratorRef, RuntimeCommand, RuntimeService, SystemClock};
use ironmaint_store::ProjectionStore;
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use tempfile::TempDir;
use time::OffsetDateTime;
use url::Url;

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");

type Service = RuntimeService<SqliteStore, NullExecutor>;

/// Open a store and a runtime over a fresh temp directory.
///
/// No adapter is registered. That is deliberate and load-bearing:
/// `CaptureCandidate` activates the candidate **outside** the
/// adapter branch (C1's notes — "whether a candidate is the active
/// one is a fact about the job, not about the adapter"), so the
/// activation this test needs happens with no adapter present. A
/// fixture adapter would make the test depend on registration order
/// to reach the very field it is checking.
async fn open() -> (TempDir, Arc<SqliteStore>, Service) {
    let tmp = TempDir::new().expect("temp dir");
    let config = SqliteStoreConfig::new(tmp.path()).with_migrations_dir(MIGRATIONS_DIR);
    let store = Arc::new(SqliteStore::open(config).await.expect("open the store"));
    let service = RuntimeService::new(
        Arc::clone(&store),
        Arc::new(SystemClock),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    );
    (tmp, store, service)
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").expect("family"),
            DistributionRelease::new("sid").expect("release"),
        ),
        PackageName::new("replay-roundtrip").expect("name"),
    )
}

fn source_candidate(job_id: JobId) -> SourceCandidate {
    let url = Url::parse("https://example.invalid/repo.git").expect("url");
    let repository = RepositoryRef::new(VcsKind::Git, url).expect("repo ref");
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).expect("commit");
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).expect("tree");
    let revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").expect("version"));
    SourceCandidate::new(
        job_id,
        revision,
        repository,
        commit,
        tree,
        OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("timestamp"),
    )
}

async fn create_job(svc: &Service) -> JobId {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("CreateJob must succeed");
    let rest = result.side_effects[0]
        .strip_prefix("job:")
        .expect("the side effect names the job")
        .split_whitespace()
        .next()
        .expect("and the id follows the prefix")
        .parse()
        .expect("and it parses");
    JobId::from_uuid(rest)
}

/// Compare the stored projection against the replayed one, field by
/// field.
///
/// This is the assertion, and it is written as a per-field report
/// rather than a single `assert_eq!` on the whole struct for two
/// reasons. A `JobProjection` diff prints five fields and the two
/// that matter are indistinguishable in it; and the whole point of
/// §36's repair tool is that it can *name* the fact the log is
/// missing, so the check that guards that tool should be able to
/// name it too.
fn assert_replay_matches(stored: &JobProjection, replayed: &JobProjection, context: &str) {
    let mut unrecoverable: Vec<&str> = Vec::new();

    if stored.job != replayed.job {
        unrecoverable.push("job");
    }
    if stored.state != replayed.state {
        unrecoverable.push("state");
    }
    if stored.active_candidate != replayed.active_candidate {
        unrecoverable.push("active_candidate");
    }
    if stored.version != replayed.version {
        unrecoverable.push("version");
    }
    if stored.updated_at != replayed.updated_at {
        unrecoverable.push("updated_at");
    }

    assert!(
        unrecoverable.is_empty(),
        "{context}: the event log does not justify the stored projection.\n\
         \n  the replay cannot recover: {}\n  \n\
         stored   : state={:?} active_candidate={:?} version={} updated_at={}\n\
         replayed : state={:?} active_candidate={:?} version={} updated_at={}\n\
         \nEvery field listed above is a fact §36 calls the log the authority for, \
         so `ironmaintctl rebuild-projections` cannot rebuild this job — and writing \
         the replay anyway would silently drop it. See doc/DEBT.md D-16.",
        unrecoverable.join(", "),
        stored.state,
        stored.active_candidate,
        stored.version,
        stored.updated_at,
        replayed.state,
        replayed.active_candidate,
        replayed.version,
        replayed.updated_at,
    );
}

/// The capture path. `CreateJob` seeds the projection and `CaptureCandidate`
/// activates the candidate, and activation is the operation D-16 is
/// about.
#[tokio::test]
async fn a_captured_job_replays_to_its_stored_projection() {
    let (_tmp, store, svc) = open().await;
    let job_id = create_job(&svc).await;

    svc.handle_command(RuntimeCommand::CaptureCandidate {
        job_id,
        candidate: source_candidate(job_id),
    })
    .await
    .expect("CaptureCandidate must succeed");

    let stored = store
        .get_projection(job_id)
        .await
        .expect("CreateJob seeds the projection");

    // The precondition, asserted rather than assumed. Without it this
    // test would pass against a build whose capture never activated —
    // the same mistake the first draft of the D-01 rebuild regression
    // test made, and the reason every assertion here states what it
    // depends on.
    assert!(
        stored.active_candidate.is_some(),
        "capture must have activated a candidate, or this test proves nothing"
    );
    assert!(
        stored.version > 0,
        "activation bumps the version in the row, or this test proves nothing"
    );

    let replayed = store
        .rebuild_projection(job_id)
        .await
        .expect("the log replays");

    assert_replay_matches(&stored, &replayed, "after capture");
}

/// The transition path, deliberately **without** a capture.
///
/// Isolating this from the test above is what makes the report
/// actionable: if this one passes, then `state` and `updated_at` are
/// both recoverable from the log and the gap is exactly the two
/// fields the capture path writes to the row and not to the log. If it
/// fails too, the gap is wider than D-16 says.
///
/// `EnterHumanReview` is used because it issues a real transition
/// through the engine with no gates to satisfy, so this test measures
/// replay rather than workflow.
#[tokio::test]
async fn a_transitioned_job_replays_to_its_stored_projection() {
    let (_tmp, store, svc) = open().await;
    let job_id = create_job(&svc).await;

    svc.handle_command(RuntimeCommand::EnterHumanReview {
        job_id,
        reason: "replay round-trip: force a Transitioned event".to_string(),
    })
    .await
    .expect("EnterHumanReview must succeed");

    let stored = store
        .get_projection(job_id)
        .await
        .expect("the transition wrote the projection");

    assert_eq!(
        stored.state,
        JobState::HumanReviewRequired,
        "the job must actually have transitioned, or this test proves nothing"
    );

    let replayed = store
        .rebuild_projection(job_id)
        .await
        .expect("the log replays");

    assert_replay_matches(&stored, &replayed, "after a transition");
}
