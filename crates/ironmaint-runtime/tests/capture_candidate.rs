//! Phase 0B.5 C1 — `RuntimeCommand::CaptureCandidate` integration
//! tests (PHASE-0B.md §6, §45).
//!
//! Each test exercises one observable contract:
//!   1. A fresh candidate is persisted into the candidate store and
//!      surfaces in `list_source_candidates_for_job`.
//!   2. `find_source_by_fingerprint` returns the same id after
//!      capture.
//!   3. A duplicate `CaptureCandidate` with the same fingerprint is
//!      idempotent — no new row, no new sequence bump.
//!   4. The handler appends a `JobEvent::Domain` audit event whose
//!      side-effect text identifies the captured candidate.
//!   5. A candidate whose `job_id` doesn't match the command's
//!      `job_id` is rejected as `InvalidInput` (preserves the
//!      `(job_id, fingerprint)` index invariant).
//!
//! `unwrap`/`expect` are allowed here because failure in a test
//! should panic; the production crate forbids them via the
//! workspace lint table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, MaintenanceEventId, PackageIdentity, PackageName,
    PackageRevision, PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::Evidence;
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{
    Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeService, SystemClock,
};
use ironmaint_state::JobEvent;
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EventStore};
use time::OffsetDateTime;
use url::Url;

// -----------------------------------------------------------------------------
// Fixtures.
// -----------------------------------------------------------------------------

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("foo").unwrap(),
    )
}

fn fixed_clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock::new(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    ))
}

fn source_candidate(job_id: JobId, version: &str) -> SourceCandidate {
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(package(), PackageVersion::new(version).unwrap());
    SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now)
}

fn build_service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

async fn create_job(svc: &RuntimeService<MockStore, NullExecutor>) -> JobId {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    parse_job_id(&result.side_effects[0])
}

fn parse_job_id(side_effect: &str) -> JobId {
    // "job:{uuid} created ..."
    let rest = side_effect
        .strip_prefix("job:")
        .expect("side-effect prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("valid uuid"))
}

// -----------------------------------------------------------------------------
// Tests.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn capture_candidate_inserts_and_advances_sequence() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    let candidate = source_candidate(job_id, "1.0.0");
    let fingerprint = candidate.fingerprint().clone();

    let result = svc
        .handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("capture");

    // CreateJob emitted sequence 1; CaptureCandidate emits sequence 2.
    assert_eq!(result.new_sequence, 2);
    assert!(
        result.side_effects[0].starts_with("source_candidate:"),
        "side effect must identify the captured candidate: {}",
        result.side_effects[0]
    );

    // The candidate must be findable by fingerprint.
    let found = store
        .find_source_by_fingerprint(&fingerprint)
        .await
        .expect("find");
    assert!(
        found.is_some(),
        "captured candidate must be findable by fingerprint"
    );

    // The candidate must appear in the per-job list.
    let ids = store
        .list_source_candidates_for_job(job_id)
        .await
        .expect("list");
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], found.unwrap());
}

#[tokio::test]
async fn capture_candidate_is_idempotent_on_duplicate_fingerprint() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    let candidate_a = source_candidate(job_id, "1.0.0");
    let fingerprint = candidate_a.fingerprint().clone();

    let first = svc
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id,
            candidate: candidate_a.clone(),
        })
        .await
        .expect("first capture");

    let second = svc
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id,
            candidate: candidate_a,
        })
        .await
        .expect("duplicate capture must not error");

    // Sequence must advance by exactly 1 between CreateJob (1) and the
    // two captures (2, 3) — duplicate must not be a no-op event.
    assert_eq!(first.new_sequence, 2);
    assert_eq!(second.new_sequence, 3);

    // But there must still be exactly one row in the per-job list.
    let ids = store
        .list_source_candidates_for_job(job_id)
        .await
        .expect("list");
    assert_eq!(
        ids.len(),
        1,
        "duplicate fingerprint must not produce a second candidate row"
    );

    // Both side-effects reference the SAME candidate id.
    assert_eq!(first.side_effects[0], second.side_effects[0]);

    // The fingerprint still resolves to that one id.
    let found = store
        .find_source_by_fingerprint(&fingerprint)
        .await
        .expect("find");
    assert_eq!(found, Some(ids[0]));
}

#[tokio::test]
async fn capture_candidate_appends_domain_event() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    let candidate = source_candidate(job_id, "1.0.0");
    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("capture");

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events");
    assert_eq!(events.len(), 2, "CreateJob + CaptureCandidate = 2 events");

    let last = events.last().expect("last event");
    assert!(
        matches!(last.event, JobEvent::Domain(_)),
        "CaptureCandidate must append a JobEvent::Domain envelope, got {:?}",
        last.event
    );
}

#[tokio::test]
async fn capture_candidate_rejects_cross_job_mismatch() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    // Build a candidate for a *different* job id.
    let other_job_id = JobId::new();
    let candidate = source_candidate(other_job_id, "1.0.0");

    let err = svc
        .handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect_err("cross-job mismatch must be rejected");

    assert_eq!(err.kind, ironmaint_runtime::RuntimeErrorKind::InvalidInput);

    // No candidate row must be persisted.
    let ids = store
        .list_source_candidates_for_job(job_id)
        .await
        .expect("list");
    assert!(
        ids.is_empty(),
        "rejected capture must not write a candidate row"
    );
}

#[tokio::test]
async fn capture_candidate_allows_two_distinct_fingerprints_for_same_job() {
    // §22 says: a job may capture multiple candidates (e.g. C1,
    // C2, C3 from the synthetic workflow). Distinct fingerprints
    // produce distinct rows.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    let c1 = source_candidate(job_id, "1.0.0");
    let c2 = source_candidate(job_id, "1.0.1");

    svc.handle_command(RuntimeCommand::CaptureCandidate {
        job_id,
        candidate: c1,
    })
    .await
    .expect("c1");
    svc.handle_command(RuntimeCommand::CaptureCandidate {
        job_id,
        candidate: c2,
    })
    .await
    .expect("c2");

    let ids = store
        .list_source_candidates_for_job(job_id)
        .await
        .expect("list");
    assert_eq!(
        ids.len(),
        2,
        "two distinct fingerprints must produce two candidate rows"
    );
}

// `_maintenance_event_id_anchor` keeps the `MaintenanceEventId` and
// `Evidence` imports live so a downstream delete of the test
// fails locally instead of in another crate.
#[allow(dead_code)]
fn _imports_anchor() -> (MaintenanceEventId, Evidence) {
    (
        MaintenanceEventId::new(),
        Evidence::new(
            CandidateFingerprint::from_hex("00".repeat(32)).unwrap(),
            ironmaint_evidence::EvidenceKind::Build,
            ironmaint_evidence::EvidenceStatus::Pass,
            ironmaint_evidence::EvidenceProducer::new("anchor"),
            ironmaint_evidence::EvidenceScope::Job(JobId::new()),
            OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        ),
    )
}

#[allow(dead_code)]
fn _system_clock_anchor() -> Arc<dyn Clock> {
    Arc::new(SystemClock)
}
