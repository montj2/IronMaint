//! Phase 0B.5 C4 — `try_transition` (§17, §18).
//!
//! Each test exercises one observable contract:
//!   1. `try_transition` rejects stale version with
//!      `RuntimeErrorKind::ConcurrentModification`.
//!   2. `try_transition` blocks when gates/obligations are missing.
//!   3. `try_transition` allows when gates pass and obligations
//!      are satisfied.
//!   4. `try_transition` emits a `JobEvent::Transitioned`
//!      audit event on success.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    AuthorityId, CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
    SourceCandidate, VcsKind,
};
use ironmaint_evidence::{EvidenceKind, GateDefinition, GateRequirement, GateStage};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_policy::{
    Applicability, Obligation, ObligationOutcome, ObligationStrength, PolicyReference,
};
use ironmaint_runtime::{Clock, FixedClock, RuntimeCommand, RuntimeErrorKind, RuntimeService};
use ironmaint_state::JobEvent as StateJobEvent;
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EventStore, GateStore, ObligationStore, ProjectionStore};
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

async fn attach_candidate(store: &Arc<MockStore>, job_id: JobId) -> CandidateFingerprint {
    // Build a real SourceCandidate and persist it so try_transition
    // can resolve `projection.active_candidate -> fingerprint`.
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

#[tokio::test]
async fn try_transition_rejects_stale_version() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;

    let err = svc
        .try_transition(job_id, JobState::Intake, 99)
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::ConcurrentModification);
}

#[tokio::test]
async fn try_transition_blocks_when_gate_missing() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;

    // No gate seeded for SourcePreparation -> missing gate blocker.
    let err = svc
        .try_transition(job_id, JobState::Intake, 0)
        .await
        .expect_err("must block");
    let msg = format!("{err}");
    assert!(
        msg.contains("missing gate"),
        "expected missing-gate blocker, got: {msg}"
    );
    let _ = fp;
}

#[tokio::test]
async fn try_transition_allows_when_gate_passes() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;

    let transition = svc
        .try_transition(job_id, JobState::Intake, 0)
        .await
        .expect("with gate satisfied, transition allowed");
    assert_eq!(transition.to, JobState::Intake);
}

#[tokio::test]
async fn try_transition_emits_state_transitioned_event() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;

    svc.try_transition(job_id, JobState::Intake, 0)
        .await
        .expect("transition");

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events");
    let transitioned = events
        .iter()
        .find(|e| matches!(e.event, StateJobEvent::Transitioned(_)))
        .expect("one transitioned event");
    if let StateJobEvent::Transitioned(applied) = &transitioned.event {
        assert_eq!(applied.transition.from, JobState::EventDetected);
        assert_eq!(applied.transition.to, JobState::Intake);
    } else {
        panic!("expected Transitioned event");
    }
}

#[tokio::test]
async fn try_transition_walks_full_chain_to_release_review() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;

    // Each transition in the §20 forward chain has one gate stage.
    let stages = [
        (GateStage::SourcePreparation, JobState::Intake),
        (GateStage::SourceAnalysis, JobState::SourceReview),
        (GateStage::IssueAnalysis, JobState::CandidateAssembly),
        (GateStage::Maintenance, JobState::SourceRevision),
        (GateStage::PolicyEvaluation, JobState::SourceIntegrity),
        (GateStage::BuildValidation, JobState::BuildValidation),
        (GateStage::PackageQa, JobState::PackageQaValidation),
        (
            GateStage::FunctionalValidation,
            JobState::FunctionalValidation,
        ),
        (GateStage::UpgradeValidation, JobState::UpgradeValidation),
        (GateStage::ReleaseReview, JobState::ReleaseReview),
    ];

    let mut version = 0u64;
    for (stage, target) in stages {
        seed_passing_gate(&store, job_id, &fp, stage).await;
        let t = svc
            .try_transition(job_id, target, version)
            .await
            .unwrap_or_else(|e| panic!("transition to {target:?} failed: {e}"));
        assert_eq!(t.to, target);
        version = projection_version(&store, job_id).await;
    }

    // ReleaseReview reached. Next: FinalValidation requires the
    // CandidateAssembly gate. Without it, must block.
    let err = svc
        .try_transition(job_id, JobState::FinalValidation, version)
        .await
        .expect_err("must block on missing gate");
    let msg = format!("{err}");
    assert!(
        msg.contains("missing gate") || msg.contains("missing obligation"),
        "expected blocker at ReleaseReview → FinalValidation, got: {msg}"
    );
}

async fn projection_version(store: &Arc<MockStore>, job_id: JobId) -> u64 {
    store
        .get_projection(job_id)
        .await
        .expect("projection")
        .version
}

#[tokio::test]
async fn try_transition_blocks_on_missing_mandatory_obligation() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;

    let obligation = Obligation::new(
        fp.clone(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "release-ready",
    )
    .expect("static literal fits");
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let at_final = walk_to_final_validation(&store, &svc, job_id, &fp).await;

    // A `NotEvaluated` mandatory obligation blocks exactly like a
    // failing one: rule 12 is `require_policy_completion: true`, so
    // `check_obligations` runs and an unsatisfied obligation is a
    // blocker regardless of why it is unsatisfied.
    let err = svc
        .try_transition(job_id, JobState::ReadyForApproval, at_final)
        .await
        .expect_err("must block on the unevaluated obligation");
    let msg = format!("{err}");
    assert!(
        msg.contains("obligation"),
        "expected an obligation blocker at FinalValidation → ReadyForApproval, got: {msg}"
    );
    let projection = store.get_projection(job_id).await.expect("projection");
    assert_eq!(
        projection.state,
        JobState::FinalValidation,
        "a blocked transition must not move the job"
    );
}

/// The gap this pins. Before `RecordObligationOutcome` a mandatory
/// obligation could only ever be `NotEvaluated` (derived) or `Pass`,
/// so `check_obligations`' `FailedObligation` branch had no production
/// way to be reached: a policy evaluation returning a negative verdict
/// was *unrepresentable*, not merely unreachable. That is 0B §101
/// step 18 — "Mandatory synthetic policy obligation fails" — and
/// without it steps 19–24 (patch, capture C4, obligation passes)
/// have nothing to repair.
#[tokio::test]
async fn a_failed_mandatory_obligation_blocks_ready_for_approval() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;

    let obligation = Obligation::new(
        fp.clone(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "reproducible-build",
    )
    .expect("static literal fits");
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let at_final = walk_to_final_validation(&store, &svc, job_id, &fp).await;

    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "reproducible-build".to_string(),
        outcome: ObligationOutcome::Fail,
    })
    .await
    .expect("record the failing verdict");

    let err = svc
        .try_transition(job_id, JobState::ReadyForApproval, at_final)
        .await
        .expect_err("a failed mandatory obligation must block");
    let msg = format!("{err}");
    assert!(
        msg.contains("obligation"),
        "expected an obligation blocker, got: {msg}"
    );

    // §101 step 24: the same obligation passes and the job advances.
    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "reproducible-build".to_string(),
        outcome: ObligationOutcome::Pass,
    })
    .await
    .expect("record the passing verdict");

    let t = svc
        .try_transition(job_id, JobState::ReadyForApproval, at_final)
        .await
        .expect("a satisfied obligation must let the job through");
    assert_eq!(t.to, JobState::ReadyForApproval);
}

/// `RequiresReview` is the third status the engine branches on, and
/// the one an approval-pending policy check returns. It must block
/// identically to `Fail`, or an obligation that needs a human to weigh
/// in would be treated as satisfied.
#[tokio::test]
async fn an_obligation_awaiting_review_blocks_ready_for_approval() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;

    let obligation = Obligation::new(
        fp.clone(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "ambiguous-maintainer-request",
    )
    .expect("static literal fits");
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let at_final = walk_to_final_validation(&store, &svc, job_id, &fp).await;

    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "ambiguous-maintainer-request".to_string(),
        outcome: ObligationOutcome::RequiresReview,
    })
    .await
    .expect("record the review verdict");

    let err = svc
        .try_transition(job_id, JobState::ReadyForApproval, at_final)
        .await
        .expect_err("RequiresReview must block, exactly as Fail does");
    assert!(
        format!("{err}").contains("obligation"),
        "expected an obligation blocker, got: {err}"
    );
}

/// A failing *recommended* obligation must not block. `check_obligations`
/// skips non-mandatory strengths, and the reporting path skips them
/// too — an agent told to fix a recommendation the state machine does
/// not require is being sent after work that changes nothing.
#[tokio::test]
async fn a_failing_recommended_obligation_does_not_block() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let fp = attach_candidate(&store, job_id).await;

    let obligation = Obligation::new(
        fp.clone(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Recommended,
        Applicability::Applicable,
        "prefer-newer-snapshot",
    )
    .expect("static literal fits");
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let at_final = walk_to_final_validation(&store, &svc, job_id, &fp).await;

    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "prefer-newer-snapshot".to_string(),
        outcome: ObligationOutcome::Fail,
    })
    .await
    .expect("record the failing verdict");

    let t = svc
        .try_transition(job_id, JobState::ReadyForApproval, at_final)
        .await
        .expect("a failing recommendation must not gate release");
    assert_eq!(t.to, JobState::ReadyForApproval);
}

/// Walk the forward chain to `FinalValidation` with every gate
/// passing, so the only thing left to block on is policy. Returns
/// the projection version to pass as `expected_version`.
async fn walk_to_final_validation(
    store: &Arc<MockStore>,
    svc: &RuntimeService<MockStore, NullExecutor>,
    job_id: JobId,
    fp: &CandidateFingerprint,
) -> u64 {
    let stages = [
        (GateStage::SourcePreparation, JobState::Intake),
        (GateStage::SourceAnalysis, JobState::SourceReview),
        (GateStage::IssueAnalysis, JobState::CandidateAssembly),
        (GateStage::Maintenance, JobState::SourceRevision),
        (GateStage::PolicyEvaluation, JobState::SourceIntegrity),
        (GateStage::BuildValidation, JobState::BuildValidation),
        (GateStage::PackageQa, JobState::PackageQaValidation),
        (
            GateStage::FunctionalValidation,
            JobState::FunctionalValidation,
        ),
        (GateStage::UpgradeValidation, JobState::UpgradeValidation),
        (GateStage::ReleaseReview, JobState::ReleaseReview),
    ];
    let mut version = 0u64;
    for (stage, target) in stages {
        seed_passing_gate(store, job_id, fp, stage).await;
        svc.try_transition(job_id, target, version)
            .await
            .unwrap_or_else(|e| panic!("transition to {target:?} failed: {e}"));
        version = projection_version(store, job_id).await;
    }
    // ReleaseReview → FinalValidation gates on CandidateAssembly.
    seed_passing_gate(store, job_id, fp, GateStage::CandidateAssembly).await;
    svc.try_transition(job_id, JobState::FinalValidation, version)
        .await
        .expect("reach FinalValidation");

    // Rule 12 (FinalValidation → ReadyForApproval) gates on its own
    // FinalValidation stage, so seed that too. After this the only
    // thing left to block is policy — which is the point.
    seed_passing_gate(store, job_id, fp, GateStage::FinalValidation).await;
    projection_version(store, job_id).await
}
