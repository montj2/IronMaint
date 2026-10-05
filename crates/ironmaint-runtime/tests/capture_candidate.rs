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
//!      side-effect text identifies the captured candidate. As of
//!      0B.10 C1 it appends *two*: capture, then the activation
//!      that capture now drives (see `adapter_plans.rs`).
//!   5. A candidate whose `job_id` doesn't match the command's
//!      `job_id` is rejected as `InvalidInput` (preserves the
//!      `(job_id, fingerprint)` index invariant).
//!   6. **D-21 closure** — a capture whose `job_id` has no projection
//!      is rejected at the boundary as typed `InvalidInput`, not
//!      silently accepted (`MockStore`) or surfaced as a raw store
//!      foreign-key violation (`SqliteStore`). The agent must call
//!      `job.create` first.
//!
//! `unwrap`/`expect` are allowed here because failure in a test
//! should panic; the production crate forbids them via the
//! workspace lint table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate,
    VcsKind,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::error::{RuntimeError, RuntimeErrorKind};
use ironmaint_runtime::{Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeService};
use ironmaint_state::JobEvent;
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EventStore, ProjectionStore};
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

    // CreateJob emitted sequences 1 and 2 (`JobCreated`, then the
    // `Domain` audit reference); CaptureCandidate emits sequence 3.
    assert_eq!(result.new_sequence, 3);
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

    // As of 0B.10 C1, capture writes *two* events the first time:
    // the capture itself, then the activation that follows it (the
    // activation appends its own `JobEvent::Domain` envelope, which
    // is what lets `rebuild_projection` reconstruct which candidate
    // was active). CreateJob is 1 and 2 (a `JobCreated` carrying
    // the seed projection, then the `Domain` reference), so the first
    // capture is 3 and 4, and the duplicate capture — which
    // re-activates nothing — is 5.
    assert_eq!(first.new_sequence, 3);
    assert_eq!(second.new_sequence, 5);

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
async fn capture_candidate_appends_a_candidate_activated_event() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    let candidate = source_candidate(job_id, "1.0.0");
    let fingerprint = candidate.fingerprint().clone();
    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("capture");

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events");
    assert_eq!(
        events.len(),
        4,
        "CreateJob's two envelopes (JobCreated, then the Domain \
         reference) plus CaptureCandidate's two (the capture and the \
         activation that follows it) = 4 events"
    );

    // The count is unchanged by D-16's fix — activation still appends
    // exactly one event. What changed is *which*: a bare `Domain` id
    // cannot be replayed, so the log recorded that something happened
    // while the row recorded what. §36 makes the log the authority,
    // and §30 binds every gate verdict to the active candidate, so a
    // replay through `Domain` silently dropped the binding for every
    // job that had ever captured one.
    let last = events.last().expect("last event");
    let JobEvent::CandidateActivated(activation) = &last.event else {
        panic!(
            "CaptureCandidate must append a JobEvent::CandidateActivated \
             envelope, got {:?}",
            last.event
        );
    };

    // And it must carry what the row got. Asserting the variant alone
    // would pass against a payload that dropped the fields — which is
    // the shape the red S7 commit caught, when it reported `version`
    // and `updated_at` as unrecoverable alongside the candidate.
    let stored = store.get_projection(job_id).await.expect("projection");
    assert_eq!(
        activation.candidate_id,
        stored.active_candidate.expect("activated"),
        "the event names the candidate the row activated"
    );
    assert_eq!(
        activation.fingerprint, fingerprint,
        "and the fingerprint that identified it"
    );
    assert_eq!(
        activation.version_after, stored.version,
        "and the version the activation's CAS left behind"
    );
    assert_eq!(
        activation.updated_at, stored.updated_at,
        "and the timestamp it stamped"
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

#[tokio::test]
async fn capture_candidate_for_unknown_job_returns_invalid_input() {
    // D-21 closure (Phase 1 §4): `candidate.capture` must never create a
    // job implicitly. The agent must call `job.create` first, and the
    // runtime must surface a typed `InvalidInput` error at the
    // capture boundary so the agent gets the same signal whether the
    // backing store enforces referential integrity (SqliteStore) or
    // does not (MockStore). Previously the mock silently accepted the
    // orphan and SQLite surfaced a raw `FOREIGN KEY constraint failed`
    // string — neither is acceptable for an autonomous agent.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());

    let unknown_job = JobId::new();
    let candidate = source_candidate(unknown_job, "1.0.0");
    let fingerprint = candidate.fingerprint().clone();

    let err = svc
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id: unknown_job,
            candidate,
        })
        .await
        .expect_err("capture against unknown job must be rejected");

    let kind = err.kind;
    assert_eq!(
        kind,
        RuntimeErrorKind::InvalidInput,
        "D-21 closure: capture must return typed InvalidInput (got {kind:?})"
    );

    // The candidate must not be persisted — the rejection is at the
    // boundary, not after a half-completed write. The mocked parity
    // case in `tests/store_parity_sqlite.rs` covers the same invariant
    // on the SQLite side via the FK constraint.
    let stored = store
        .find_source_by_fingerprint(&fingerprint)
        .await
        .expect("store read");
    assert!(
        stored.is_none(),
        "rejected capture must leave no candidate row behind"
    );

    // Touch the value so the import is not flagged as unused.
    let _ = RuntimeError::new(kind, "unused");
}
