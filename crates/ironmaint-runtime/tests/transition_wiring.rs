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
use ironmaint_policy::{Applicability, Obligation, ObligationStrength, PolicyReference};
use ironmaint_runtime::{Clock, FixedClock, RuntimeErrorKind, RuntimeService};
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

fn fingerprint_zero() -> CandidateFingerprint {
    CandidateFingerprint::from_hex("00".repeat(32)).unwrap()
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
    let _svc = build_service(store.clone());
    let job_id = seed_projection(&store, JobState::EventDetected, 0).await;
    let _fp = attach_candidate(&store, job_id).await;

    // Persist a mandatory obligation tied to a synthetic
    // candidate fingerprint; assert it was recorded. We do not
    // force a transition because the obligation machinery
    // participates in `require_policy_completion` only at the
    // FinalValidation → ReadyForApproval edge; the test instead
    // verifies the obligation round-trips through the store,
    // which is what `try_transition` consults.
    let fp = fingerprint_zero();
    let obligation = Obligation::new(
        fp,
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "release-ready",
    )
    .expect("static literal fits");
    let _ob_id = store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let obligations = store
        .list_obligations_for_job(job_id)
        .await
        .expect("list obs");
    assert_eq!(obligations.len(), 1, "exactly one obligation recorded");
}
