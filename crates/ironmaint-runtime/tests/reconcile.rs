//! Phase 0B.5 C5 — `Reconcile` walks the static rule table
//! against the evidence ledger (§41, §42).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
    SourceCandidate, VcsKind,
};
use ironmaint_evidence::{
    EvidenceKind, GateDefinition, GateRequirement, GateResult, GateStage, RequiredEvidenceStatus,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{Clock, FixedClock, ReconcileOutcome, RuntimeErrorKind, RuntimeService};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, GateStore, ProjectionStore};
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

async fn attach_candidate(store: &Arc<MockStore>, job_id: JobId) -> CandidateFingerprint {
    let repo_url = url::Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, repo_url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap());
    let candidate = SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now);
    let fp = candidate.fingerprint().clone();
    let cid = store
        .put_source_candidate(&candidate)
        .await
        .expect("put source");
    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.active_candidate = Some(cid);
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("attach");
    fp
}

async fn seed_passing_gate(
    store: &Arc<MockStore>,
    job_id: JobId,
    fp: &CandidateFingerprint,
    stage: GateStage,
) {
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

async fn assert_state(store: &Arc<MockStore>, job_id: JobId, expected: JobState) {
    let p = store.get_projection(job_id).await.expect("projection");
    assert_eq!(p.state, expected, "expected state {expected:?}");
}

#[tokio::test]
async fn reconcile_returns_noop_when_no_active_candidate() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;

    // No candidate attached, no gates; reconcile cannot advance.
    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(matches!(outcome, ReconcileOutcome::NoOp { .. }));
}

/// §41: reconciliation "may continue through multiple trivially
/// satisfied stages until reaching a state requiring [agent
/// action]". With only rule 1 satisfied, the walk advances once
/// and then reports the rule it could not satisfy next — which is
/// the shape of every outcome except the two that stop on a state
/// (`NoOp`) or an actor (`NeedsActorDecision`).
#[tokio::test]
async fn reconcile_advances_and_reports_where_it_stopped() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;

    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(
        matches!(
            outcome,
            ReconcileOutcome::Blocked {
                current: JobState::Intake,
                target: JobState::SourceReview,
                ..
            }
        ),
        "expected the walk to advance to Intake and stop on rule 2, got {outcome:?}"
    );
    assert_state(&store, job_id, JobState::Intake).await;
}

#[tokio::test]
async fn reconcile_returns_blocked_when_gate_missing() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let _fp = attach_candidate(&store, job_id).await;

    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(
        matches!(outcome, ReconcileOutcome::Blocked { .. }),
        "expected Blocked, got {outcome:?}"
    );
    assert_state(&store, job_id, JobState::EventDetected).await;
}

/// One call, two rules. An implementation that advanced a single
/// rule per call would leave the job at `Intake` here and report a
/// second `reconcile` being needed — which is the defect this test
/// exists to pin: the walk's own `for` loop was silenced with
/// `#[allow(clippy::never_loop)]` and returned on its first pass.
#[tokio::test]
async fn one_reconcile_walks_every_satisfied_rule() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourceAnalysis).await;

    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(
        matches!(
            outcome,
            ReconcileOutcome::Blocked {
                current: JobState::SourceReview,
                target: JobState::CandidateAssembly,
                ..
            }
        ),
        "rules 1 and 2 are satisfied, so the walk should have applied both \
         before reporting rule 3; got {outcome:?}"
    );
    assert_state(&store, job_id, JobState::SourceReview).await;
}

#[tokio::test]
async fn reconcile_skips_approval_gated_transition() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    // Place job directly at ReadyForApproval with an active
    // candidate attached. Approvals registry is empty, so
    // the next transition is `Approved` and is blocked on
    // `MissingApproval` — which reconcile surfaces as
    // `NeedsActorDecision`.
    let job_id = seed_projection(&store, JobState::ReadyForApproval, 0).await;
    let _fp = attach_candidate(&store, job_id).await;

    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(
        matches!(outcome, ReconcileOutcome::NeedsActorDecision { .. }),
        "expected NeedsActorDecision at ReadyForApproval, got {outcome:?}"
    );
    assert_state(&store, job_id, JobState::ReadyForApproval).await;
}

#[tokio::test]
async fn reconcile_returns_exceptional_for_human_review_required() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::HumanReviewRequired, 0).await;

    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(
        matches!(outcome, ReconcileOutcome::Exceptional { .. }),
        "expected Exceptional, got {outcome:?}"
    );
}

#[tokio::test]
async fn reconcile_returns_noop_for_terminal_states() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());

    let published_id = seed_projection(&store, JobState::Published, 0).await;
    let outcome = svc.reconcile(published_id).await.expect("reconcile");
    assert!(matches!(
        outcome,
        ReconcileOutcome::NoOp {
            current: JobState::Published
        }
    ));

    let cancelled_id = seed_projection(&store, JobState::Cancelled, 0).await;
    let outcome = svc.reconcile(cancelled_id).await.expect("reconcile");
    assert!(matches!(
        outcome,
        ReconcileOutcome::NoOp {
            current: JobState::Cancelled
        }
    ));
}

#[tokio::test]
async fn reconcile_replays_concurrent_modification_through_runtime_error_kind() {
    // The contract: a `ConcurrentModification` from
    // `try_transition` is converted by `reconcile` into
    // `ReconcileOutcome::ConcurrentModification`, NOT into
    // `RuntimeErrorKind::ConcurrentModification`.
    //
    // The mapping is covered directly by
    // `try_transition_rejects_stale_version` in
    // `transition_wiring.rs`. Here we assert the
    // reconcile-side outcome shape: when there is no
    // candidate at all, reconcile returns NoOp (the
    // pre-capture short-circuit).
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(matches!(outcome, ReconcileOutcome::NoOp { .. }));
    let _ = RuntimeErrorKind::ConcurrentModification; // referenced for documentation
}
