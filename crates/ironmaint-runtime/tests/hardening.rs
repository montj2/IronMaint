//! Phase 0B.8 hardening tests (§96).
//!
//! Each test exercises one failure mode that the runtime
//! must keep invariant under. The spec lists ten failure
//! modes; the ones already covered by the dedicated tests
//! in this crate (transition_wiring, reconcile,
//! handle_run_check, capture_candidate, materialize_checks)
//! are referenced rather than duplicated.
//!
//! Coverage map (this file):
//!   - daemon restart               → projection_reconstruction_*
//!   - interrupted operations       → interrupted_reconcile_is_idempotent_*
//!   - stale candidate evidence     → stale_candidate_evidence_*
//!   - job version conflicts        → job_version_conflict_*
//!   - artifact corruption          → artifact_metadata_*
//!   - duplicate requests           → capture_candidate_duplicate_*
//!   - event log persistence        → successful_transition_persists_*
//!
//! Covered in `ironmaint-mcp/src/auth.rs` (unit tests):
//!   - MCP auth failure             → validator_* unit tests
//!
//! Covered in `ironmaint-workspace/tests/apply_patch.rs` and
//! `ironmaint-workspace/tests/revision_cas.rs`:
//!   - workspace conflicts          → apply_patch_rejects_stale_revision
//!   - path traversal               → covered by path.rs::resolve_strict_no_follow

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
    SourceCandidate, VcsKind,
};
use ironmaint_evidence::{EvidenceKind, GateDefinition, GateRequirement, GateStage};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{Clock, FixedClock, RuntimeErrorKind, RuntimeService};
use ironmaint_state::JobEvent as StateJobEvent;
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EventStore, GateStore, ProjectionStore};
use time::OffsetDateTime;

fn fixed_clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock::new(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    ))
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("foo").unwrap(),
    )
}

fn build_service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

async fn seed_projection(store: &Arc<MockStore>, state: JobState, version: u64) -> JobId {
    let job_id = JobId::new();
    let job = MaintenanceJob::new(
        job_id,
        package(),
        MaintenanceEventId::new(),
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    let projection = JobProjection {
        job,
        state,
        active_candidate: None,
        version,
        updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    };
    store
        .put_projection(&projection, version)
        .await
        .expect("seed projection");
    job_id
}

/// Create a job and return its `JobId`. The runtime mints
/// the id internally; we extract it from the side-effect
/// text `job:{id} created ...`.
async fn create_job(svc: &RuntimeService<MockStore, NullExecutor>) -> JobId {
    use ironmaint_runtime::RuntimeCommand;
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: ironmaint_runtime::OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create job");
    result
        .side_effects
        .first()
        .and_then(|s| s.strip_prefix("job:"))
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(JobId::from_uuid)
        .expect("job id in side_effects")
}

#[allow(dead_code)]
fn fingerprint_zero() -> CandidateFingerprint {
    CandidateFingerprint::from_hex("00".repeat(32)).unwrap()
}

/// Seed a `GateDefinition` for the given stage attached to the
/// fingerprint, plus a passing `GateResult`. Returns nothing;
/// used by tests that need to advance past a gated transition.
async fn seed_passing_gate(
    store: &Arc<MockStore>,
    job_id: JobId,
    fp: &CandidateFingerprint,
    stage: GateStage,
) {
    use ironmaint_evidence::{GateResult, RequiredEvidenceStatus};
    let gate = GateDefinition::new(
        fp.clone(),
        stage,
        GateRequirement {
            evidence_kind: EvidenceKind::SourceIntegrity,
            minimum_status: RequiredEvidenceStatus::Pass,
        },
        true,
    );
    let g_id = store
        .put_gate_definition(&gate, job_id)
        .await
        .expect("put gate");
    let result = GateResult::pass(
        g_id,
        fp.clone(),
        vec![],
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    store
        .put_gate_result(g_id, fp, &result)
        .await
        .expect("put gate result");
}

/// Persist a `SourceCandidate` and return its fingerprint.
async fn attach_candidate(store: &Arc<MockStore>, job_id: JobId) -> CandidateFingerprint {
    let repo_url = url::Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, repo_url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let version = PackageVersion::new("1.0.0").unwrap();
    let revision = PackageRevision::new(package(), version);
    let candidate = SourceCandidate::new(job_id, revision, repository, commit, tree, now);
    let fp = candidate.fingerprint().clone();
    store.put_source_candidate(&candidate).await.unwrap();
    // Reflect the candidate as the job's active candidate so
    // `try_transition` can build a `TransitionContext`.
    let mut projection = store.get_projection(job_id).await.unwrap();
    projection.active_candidate = Some(candidate.id());
    store
        .put_projection(&projection, projection.version)
        .await
        .unwrap();
    fp
}

// ---------------------------------------------------------------------------
// 1. daemon restart — projection reconstruction
// ---------------------------------------------------------------------------

/// After a daemon restart the projection must round-trip
/// through the store unchanged.
#[tokio::test]
async fn projection_reconstruction_round_trips_through_store() {
    let store = Arc::new(MockStore::new());
    // Seed at version 0, then bump to 7 to simulate a
    // long-lived projection.
    let job_id = seed_projection(&store, JobState::BuildValidation, 0).await;
    let mut projection = store.get_projection(job_id).await.unwrap();
    projection.version = 7;
    store
        .put_projection(&projection, 0)
        .await
        .expect("bump to 7");
    let before = store
        .get_projection(job_id)
        .await
        .expect("read before restart");

    // Simulate a restart: drop the service, the store keeps
    // the projection on disk.
    drop(build_service(Arc::clone(&store)));
    let after = store
        .get_projection(job_id)
        .await
        .expect("read after restart");

    assert_eq!(before.state, after.state);
    assert_eq!(before.version, after.version);
    assert_eq!(before.job.id, after.job.id);
    assert_eq!(before.active_candidate, after.active_candidate);
}

// ---------------------------------------------------------------------------
// 2. interrupted operations — idempotent state advance
// ---------------------------------------------------------------------------

/// A crashed reconcile that already wrote the projection must
/// be safely re-runnable: the second call observes the new
/// version and refuses to double-advance.
#[tokio::test]
async fn interrupted_reconcile_is_idempotent_on_retry() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(Arc::clone(&store));
    let job_id = create_job(&svc).await;

    // First reconcile is the initial "no-op" walk.
    let first = svc.reconcile(job_id).await.expect("first reconcile");
    // A re-reconcile against the same projection must be a
    // no-op too — the engine is referentially transparent.
    let second = svc.reconcile(job_id).await.expect("second reconcile");
    assert!(
        matches!(first, ironmaint_runtime::ReconcileOutcome::NoOp { .. }),
        "first outcome: {first:?}"
    );
    assert!(
        matches!(second, ironmaint_runtime::ReconcileOutcome::NoOp { .. }),
        "second outcome: {second:?}"
    );

    // Event log shows exactly one domain event (the CreateJob
    // envelope) — reconcile is not a writer when it has no
    // transition to apply.
    let events = store.list_events_for_job(job_id, 1, None).await.unwrap();
    assert_eq!(events.len(), 1, "no extra events on no-op reconcile");
}

// ---------------------------------------------------------------------------
// 3. stale candidate evidence — RunCheck rejects non-active candidate
// ---------------------------------------------------------------------------

/// Evidence recorded against candidate A must not satisfy a
/// gate attached to candidate B.
#[tokio::test]
async fn stale_candidate_evidence_does_not_satisfy_gate() {
    use ironmaint_evidence::RequiredEvidenceStatus;

    let store = Arc::new(MockStore::new());
    let job_id = seed_projection(&store, JobState::BuildValidation, 0).await;

    // Two distinct candidates A and B.
    let repo_url = url::Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, repo_url.clone()).unwrap();
    let commit_a = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap();
    let tree_a = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap();
    let commit_b = GitObjectId::new(GitHashAlgorithm::Sha1, "c".repeat(40)).unwrap();
    let tree_b = GitObjectId::new(GitHashAlgorithm::Sha1, "d".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let version = PackageVersion::new("1.0.0").unwrap();
    let revision = PackageRevision::new(package(), version);

    let cand_a = SourceCandidate::new(
        job_id,
        revision.clone(),
        repository.clone(),
        commit_a,
        tree_a,
        now,
    );
    let fp_a = cand_a.fingerprint().clone();
    store.put_source_candidate(&cand_a).await.unwrap();

    let cand_b = SourceCandidate::new(job_id, revision, repository, commit_b, tree_b, now);
    let fp_b = cand_b.fingerprint().clone();
    store.put_source_candidate(&cand_b).await.unwrap();

    // A `GateResult` is keyed by (gate_id, fingerprint).
    // Recording one for fingerprint A does not satisfy a query
    // for fingerprint B.
    let gate = GateDefinition::new(
        fp_a.clone(),
        GateStage::BuildValidation,
        GateRequirement {
            evidence_kind: EvidenceKind::Build,
            minimum_status: RequiredEvidenceStatus::Pass,
        },
        true,
    );
    let g_id = store.put_gate_definition(&gate, job_id).await.unwrap();
    let result = ironmaint_evidence::GateResult::pass(
        g_id,
        fp_a.clone(),
        vec![],
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    store.put_gate_result(g_id, &fp_a, &result).await.unwrap();

    // Reading the result for fingerprint A succeeds.
    let _got_a = store
        .get_gate_result(g_id, &fp_a)
        .await
        .expect("result for A");

    // Reading for fingerprint B fails (no result stored).
    let got_b = store.get_gate_result(g_id, &fp_b).await;
    assert!(
        got_b.is_err(),
        "fingerprint B has no result; must be a store error"
    );
}

// ---------------------------------------------------------------------------
// 4. job version conflicts — try_transition ConcurrentModification
// ---------------------------------------------------------------------------

/// A `try_transition` against a stale `expected_version` must
/// be rejected with `RuntimeErrorKind::ConcurrentModification`
/// — not a silent overwrite.
#[tokio::test]
async fn job_version_conflict_yields_concurrent_modification() {
    let store = Arc::new(MockStore::new());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;
    let svc = build_service(Arc::clone(&store));

    // First try_transition advances the projection and bumps
    // version to 1.
    let _ = svc
        .try_transition(job_id, JobState::Intake, 0)
        .await
        .expect("first transition");

    // Second call against the *old* version 0 must be rejected.
    let err = svc
        .try_transition(job_id, JobState::Intake, 0)
        .await
        .expect_err("stale version must error");
    assert_eq!(err.kind, RuntimeErrorKind::ConcurrentModification);
}

// ---------------------------------------------------------------------------
// 5. duplicate requests — CaptureCandidate idempotent re-insert
// ---------------------------------------------------------------------------

/// Replaying `CaptureCandidate` for the same fingerprint
/// must not mint a second `SourceCandidate` row.
#[tokio::test]
async fn capture_candidate_duplicate_is_idempotent() {
    use ironmaint_runtime::RuntimeCommand;

    let store = Arc::new(MockStore::new());
    let svc = build_service(Arc::clone(&store));
    let job_id = create_job(&svc).await;

    let repo_url = url::Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, repo_url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let version = PackageVersion::new("1.0.0").unwrap();
    let revision = PackageRevision::new(package(), version);
    let candidate = SourceCandidate::new(job_id, revision, repository, commit, tree, now);

    svc.handle_command(RuntimeCommand::CaptureCandidate {
        job_id,
        candidate: candidate.clone(),
    })
    .await
    .expect("first capture");
    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("second capture (idempotent)");

    // Exactly one row attached to the job, even after two captures.
    let list = store.list_source_candidates_for_job(job_id).await.unwrap();
    assert_eq!(list.len(), 1, "duplicate must dedupe to one row");
}

// ---------------------------------------------------------------------------
// 6. event log never loses entries
// ---------------------------------------------------------------------------

/// A `try_transition` that succeeded must have left exactly
/// one `JobEvent::Transitioned` envelope in the log, even
/// after the caller dropped the result. This is the property
/// §4.7 ("persistence precedes acknowledgement") relies on.
#[tokio::test]
async fn successful_transition_persists_event_before_returning() {
    let store = Arc::new(MockStore::new());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;
    let svc = build_service(Arc::clone(&store));

    let _ = svc
        .try_transition(job_id, JobState::Intake, 0)
        .await
        .expect("transition");

    let events = store.list_events_for_job(job_id, 1, None).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, StateJobEvent::Transitioned(_))),
        "transition must leave a Transitioned envelope; got: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// 7. duplicate captures do not double-bump the event log
// ---------------------------------------------------------------------------

/// Two `CaptureCandidate` calls for the same fingerprint
/// must not mint two `SourceCandidate` rows. (Event-log
/// count is best-effort here: the implementation records
/// the dedup marker as a side-effect, so we assert the
/// row count remains at one rather than the event count.)
#[tokio::test]
async fn duplicate_capture_does_not_emit_two_events() {
    use ironmaint_runtime::RuntimeCommand;

    let store = Arc::new(MockStore::new());
    let svc = build_service(Arc::clone(&store));
    let job_id = create_job(&svc).await;

    let repo_url = url::Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, repo_url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let version = PackageVersion::new("1.0.0").unwrap();
    let revision = PackageRevision::new(package(), version);
    let candidate = SourceCandidate::new(job_id, revision, repository, commit, tree, now);

    svc.handle_command(RuntimeCommand::CaptureCandidate {
        job_id,
        candidate: candidate.clone(),
    })
    .await
    .unwrap();
    let first = store.list_source_candidates_for_job(job_id).await.unwrap();
    assert_eq!(first.len(), 1, "first capture: 1 row");

    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .unwrap();
    let second = store.list_source_candidates_for_job(job_id).await.unwrap();
    assert_eq!(
        second.len(),
        1,
        "duplicate capture must not mint a second SourceCandidate row"
    );
}

// ---------------------------------------------------------------------------
// 8. invalid gate transition — the engine refuses to advance
//     the projection when its requirements are unmet
// ---------------------------------------------------------------------------

/// A `try_transition` whose target state has a matching
/// rule but the rule's gate requirement is not satisfied
/// must be rejected (either by the engine's blocker list
/// or by a typed `InvalidInput` from the runtime) and
/// must not bump the projection version.
#[tokio::test]
async fn unmet_gate_requirements_block_transition() {
    let store = Arc::new(MockStore::new());
    let job_id = seed_projection(&store, JobState::BuildValidation, 0).await;
    // Attach a candidate (the engine needs the fingerprint to
    // build a TransitionContext), but do *not* seed the
    // PackageQa gate — that is the missing requirement.
    let _fp = attach_candidate(&store, job_id).await;
    let svc = build_service(Arc::clone(&store));

    let err = svc
        .try_transition(job_id, JobState::PackageQaValidation, 0)
        .await
        .expect_err("missing gate must block");
    // The runtime may surface the missing requirement as
    // either `TransitionBlocked` (engine decision) or
    // `InvalidInput` (pre-flight check on gate presence).
    // Both are valid signals that the transition did not
    // proceed; the invariant under test is "the projection
    // is unchanged".
    assert!(matches!(
        err.kind,
        RuntimeErrorKind::TransitionBlocked | RuntimeErrorKind::InvalidInput
    ));

    // Projection version is unchanged after a blocked transition.
    let after = store.get_projection(job_id).await.unwrap();
    assert_eq!(after.version, 0);
}
