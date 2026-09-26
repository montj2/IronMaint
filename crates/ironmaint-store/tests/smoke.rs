//! Smoke tests for the 9 sub-traits against `MockStore`.
//!
//! Each test exercises one sub-trait end-to-end: write, read back,
//! list, and (where applicable) optimistic-concurrency conflict.
//!
//! The engine is not exercised here; smoke tests construct
//! `StateTransitioned` directly because the engine's `TransitionContext`
//! surface is wider than what a unit smoke test should pin down.

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, IssueActionId, JobId, JobProjection, JobState,
    MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{
    Evidence, EvidenceKind, EvidenceProducer, EvidenceScope, EvidenceStatus, GateDefinition,
    GateRequirement, GateResult, GateStage, RequiredEvidenceStatus,
};
use ironmaint_policy::{
    Applicability, ApprovalCategory, ApprovalRequirement, AuthorizationState, Obligation,
    ObligationStatus, ObligationStrength, PolicyBaseline, PolicyReference, PrivilegedOperation,
    PrivilegedOperationKind,
};
use ironmaint_state::{JobEvent, ProjectionApply, StateTransitioned, Transition};
use time::macros::datetime;
use url::Url;

use ironmaint_store::artifact::ArtifactRecord;
use ironmaint_store::mock::{MockStore, store};
use ironmaint_store::workspace::{WorkspaceHandle, WorkspaceState};
use ironmaint_store::{
    ArtifactMetadataStore, CandidateStore, EventStore, EvidenceStore, GateStore, IronMaintStore,
    ObligationStore, OperationStore, ProjectionStore, StoreErrorKind, WorkspaceMetadataStore,
};

// -----------------------------------------------------------------------------
// Fixtures.
// -----------------------------------------------------------------------------

fn job_id() -> JobId {
    JobId::new()
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

fn fingerprint() -> CandidateFingerprint {
    CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
}

fn source_candidate_for(job: JobId) -> SourceCandidate {
    let repo = RepositoryRef::new(
        VcsKind::Git,
        Url::parse("https://example.invalid/foo.git").unwrap(),
    )
    .unwrap();
    SourceCandidate::new(
        job,
        PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap()),
        repo,
        GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap(),
        GitObjectId::new(GitHashAlgorithm::Sha1, "c".repeat(40)).unwrap(),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

fn initial_projection(state: JobState, version: u64) -> JobProjection {
    JobProjection {
        job: MaintenanceJob::new(
            job_id(),
            package(),
            MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        ),
        state,
        active_candidate: None,
        version,
        updated_at: datetime!(2026-01-01 00:00:00 UTC),
    }
}

fn transitioned_event() -> StateTransitioned {
    StateTransitioned::new(
        Transition {
            from: JobState::EventDetected,
            to: JobState::Intake,
            rule_index: 0,
        },
        initial_projection(JobState::Intake, 1),
        datetime!(2026-01-02 00:00:00 UTC),
    )
}

fn evidence() -> Evidence {
    Evidence::new(
        fingerprint(),
        EvidenceKind::Build,
        EvidenceStatus::Pass,
        EvidenceProducer::new("ironmaint-fixture"),
        EvidenceScope::Job(job_id()),
        datetime!(2026-01-02 00:00:00 UTC),
    )
}

fn gate_definition() -> GateDefinition {
    GateDefinition::new(
        fingerprint(),
        GateStage::BuildValidation,
        GateRequirement::new(EvidenceKind::Build, RequiredEvidenceStatus::Pass),
        true,
    )
}

fn obligation() -> Obligation {
    Obligation::new(
        fingerprint(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "lintian passes",
    )
    .expect("static literal fits within REQUIREMENT_MAX")
}

fn operation() -> PrivilegedOperation {
    PrivilegedOperation::proposed(
        PrivilegedOperationKind::CanonicalRepositoryPush,
        fingerprint(),
    )
}

fn approval_requirement() -> ApprovalRequirement {
    ApprovalRequirement::new(
        fingerprint(),
        ApprovalCategory::Publication,
        "approve before push",
    )
    .expect("static literal fits within DESCRIPTION_MAX")
}

// -----------------------------------------------------------------------------
// EventStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn event_store_round_trip() {
    let s: MockStore = store();
    let jid = job_id();
    let transitioned = transitioned_event();

    let env = ironmaint_store::EventEnvelope::new(
        MaintenanceEventId::new(),
        jid,
        s.next_sequence(jid).await.unwrap(),
        datetime!(2026-01-02 00:00:00 UTC),
        JobEvent::Transitioned(transitioned),
    );

    s.append_event(&env).await.unwrap();
    let got = s.get_event(jid, 1).await.unwrap();
    assert_eq!(got, env);

    let listed = s.list_events_for_job(jid, 1, None).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sequence, 1);

    assert_eq!(s.next_sequence(jid).await.unwrap(), 2);
}

#[tokio::test]
async fn event_store_rejects_out_of_order_sequence() {
    let s: MockStore = store();
    let jid = job_id();
    let env = ironmaint_store::EventEnvelope::new(
        MaintenanceEventId::new(),
        jid,
        5,
        datetime!(2026-01-02 00:00:00 UTC),
        JobEvent::Domain(ironmaint_core::DomainEventId::new()),
    );
    let err = s.append_event(&env).await.unwrap_err();
    assert_eq!(*err.kind(), StoreErrorKind::SequenceOutOfRange);
}

// -----------------------------------------------------------------------------
// ProjectionStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn projection_store_round_trip_with_cas() {
    let s: MockStore = store();
    let jid = job_id();
    let transitioned = transitioned_event();

    // Seed by appending an event, then rebuild.
    let env = ironmaint_store::EventEnvelope::new(
        MaintenanceEventId::new(),
        jid,
        s.next_sequence(jid).await.unwrap(),
        datetime!(2026-01-02 00:00:00 UTC),
        JobEvent::Transitioned(transitioned),
    );
    s.append_event(&env).await.unwrap();

    // Rebuild then put with CAS.
    let rebuilt = s.rebuild_projection(jid).await.unwrap();
    assert_eq!(rebuilt.state, JobState::Intake);
    assert_eq!(rebuilt.version, 1);
    // First seed: expected_version=0 because no projection exists yet.
    s.put_projection(&rebuilt, 0).await.unwrap();

    // Stale write → Conflict (expected 1, found 1+). Bump to 2 first.
    let mut next = rebuilt.clone();
    next.version = 2;
    s.put_projection(&next, 1).await.unwrap();
    let mut stale = next.clone();
    stale.version = 3;
    let err = s.put_projection(&stale, 1).await.unwrap_err();
    assert_eq!(*err.kind(), StoreErrorKind::Conflict);
}

// -----------------------------------------------------------------------------
// CandidateStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn candidate_store_round_trip() {
    let s: MockStore = store();
    let jid = job_id();
    let cand = source_candidate_for(jid);
    let fp = cand.fingerprint().clone();

    let id = s.put_source_candidate(&cand).await.unwrap();
    assert_eq!(id, cand.id());

    let got = s.get_source_candidate(id).await.unwrap();
    assert_eq!(got, cand);

    s.set_active_source_candidate(jid, id).await.unwrap();
    let active = s.active_source_candidate(jid).await.unwrap();
    assert_eq!(active, Some(id));

    let by_fp = s.find_source_by_fingerprint(&fp).await.unwrap();
    assert_eq!(by_fp, Some(id));

    let list = s.list_source_candidates_for_job(jid).await.unwrap();
    assert_eq!(list, vec![id]);
}

// -----------------------------------------------------------------------------
// EvidenceStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn evidence_store_round_trip() {
    let s: MockStore = store();
    let jid = job_id();
    let ev = evidence();
    let id = s.put_evidence(&ev, jid).await.unwrap();
    assert_eq!(id, ev.id);

    let got = s.get_evidence(id).await.unwrap();
    assert_eq!(got, ev);

    let by_job = s.list_evidence_for_job(jid).await.unwrap();
    assert_eq!(by_job, vec![ev.clone()]);

    let by_fp = s.list_evidence_for_candidate(&ev.candidate).await.unwrap();
    assert_eq!(by_fp, vec![ev]);
}

// -----------------------------------------------------------------------------
// GateStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn gate_store_round_trip() {
    let s: MockStore = store();
    let jid = job_id();
    let gate = gate_definition();
    let id = s.put_gate_definition(&gate, jid).await.unwrap();
    assert_eq!(id, gate.id);

    let got = s.get_gate_definition(id).await.unwrap();
    assert_eq!(got, gate);

    let result = GateResult::pass(
        id,
        gate.candidate.clone(),
        Vec::new(),
        datetime!(2026-01-02 00:00:00 UTC),
    );
    s.put_gate_result(id, &gate.candidate, &result)
        .await
        .unwrap();
    let got_result = s.get_gate_result(id, &gate.candidate).await.unwrap();
    assert_eq!(got_result, result);

    let list = s.list_gates_for_job(jid).await.unwrap();
    assert_eq!(list, vec![id]);
}

// -----------------------------------------------------------------------------
// ObligationStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn obligation_store_round_trip() {
    let s: MockStore = store();
    let jid = job_id();
    let ob = obligation();
    let id = s.put_obligation(&ob, jid).await.unwrap();
    assert_eq!(id, ob.id);

    let got = s.get_obligation(id).await.unwrap();
    assert_eq!(got, ob);

    let list = s.list_obligations_for_job(jid).await.unwrap();
    assert_eq!(list, vec![id]);

    let mut updated = ob.clone();
    updated.status = ObligationStatus::Pass;
    s.update_obligation(id, &updated).await.unwrap();
    let got = s.get_obligation(id).await.unwrap();
    assert_eq!(got.status, ObligationStatus::Pass);
}

// -----------------------------------------------------------------------------
// OperationStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn operation_store_round_trip() {
    let s: MockStore = store();
    let jid = job_id();
    let op = operation();
    let id = s.put_operation(&op, jid).await.unwrap();
    assert_eq!(id, op.id);

    let got = s.get_operation(id).await.unwrap();
    assert_eq!(got, op);

    // Move to Executing; should appear in `list_executing_operations`.
    let mut executing = op.clone();
    executing.authorization = AuthorizationState::Executing;
    s.update_operation(id, &executing).await.unwrap();
    let executing_list = s.list_executing_operations().await.unwrap();
    assert_eq!(executing_list, vec![id]);

    // Move to Succeeded; should disappear from executing list.
    let mut succeeded = executing.clone();
    succeeded.authorization = AuthorizationState::Succeeded;
    s.update_operation(id, &succeeded).await.unwrap();
    let after = s.list_executing_operations().await.unwrap();
    assert!(after.is_empty());

    let by_job = s.list_operations_for_job(jid).await.unwrap();
    assert_eq!(by_job, vec![id]);
}

#[test]
fn approval_requirement_fixture_constructs() {
    // Sanity: the ApprovalRequirement fixture is used by future
    // runtime tests; ensure it constructs under the current policy
    // API so a downstream regression surfaces here, not in commit 11.
    let _ = approval_requirement();
}

#[test]
fn unused_field_issue_action_id_silences_warning() {
    // IssueActionId is imported for future use by the runtime; ensure
    // the import is live in this module.
    let _: IssueActionId = ironmaint_core::IssueActionId::new();
}

// -----------------------------------------------------------------------------
// ArtifactMetadataStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn artifact_metadata_round_trip() {
    let s: MockStore = store();
    let jid = job_id();
    let rec = ArtifactRecord {
        id: ironmaint_core::ArtifactId::new(),
        job_id: jid,
        producer: "ironmaint-fixture".to_string(),
        content_kind: "build-log".to_string(),
        bytes: 1024,
        stored_at: datetime!(2026-01-02 00:00:00 UTC),
    };
    let id = s.put_artifact(&rec).await.unwrap();
    assert_eq!(id, rec.id);

    let got = s.get_artifact(id).await.unwrap();
    assert_eq!(got, rec);

    assert!(s.artifact_exists(id).await.unwrap());

    let list = s.list_artifacts_for_job(jid).await.unwrap();
    assert_eq!(list, vec![id]);
}

// -----------------------------------------------------------------------------
// WorkspaceMetadataStore.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn workspace_metadata_round_trip_with_cas() {
    let s: MockStore = store();
    let jid = job_id();
    let handle: WorkspaceHandle = "ws-1".to_string();
    let st = WorkspaceState {
        handle: handle.clone(),
        job_id: jid,
        revision: 0,
        base_candidate: None,
        dirty: false,
        created_at: time::OffsetDateTime::now_utc(),
        updated_at: time::OffsetDateTime::now_utc(),
    };

    // First write requires expected_revision=0.
    s.put_workspace_state(&st, 0).await.unwrap();
    let got = s.get_workspace_state(&handle).await.unwrap();
    assert_eq!(got, st);

    // CAS bump.
    let mut next = st.clone();
    next.revision = 1;
    s.put_workspace_state(&next, 0).await.unwrap(); // expected=0 (current)
    let got = s.get_workspace_state(&handle).await.unwrap();
    assert_eq!(got.revision, 1);

    // Stale write → Conflict.
    let mut stale = next.clone();
    stale.revision = 2;
    let err = s.put_workspace_state(&stale, 0).await.unwrap_err();
    assert_eq!(*err.kind(), StoreErrorKind::Conflict);

    let list = s.list_workspaces_for_job(jid).await.unwrap();
    assert_eq!(list, vec![handle]);
}

// -----------------------------------------------------------------------------
// ProjectionApply (sanity check that the extension trait works through the
// state crate; mirrors what the store uses during rebuild).
// -----------------------------------------------------------------------------

#[test]
fn projection_apply_via_state_crate() {
    let proj = initial_projection(JobState::EventDetected, 0);
    let occurred = datetime!(2026-01-02 00:00:00 UTC);
    let transitioned = StateTransitioned::new(
        Transition {
            from: JobState::EventDetected,
            to: JobState::Intake,
            rule_index: 0,
        },
        initial_projection(JobState::Intake, 1),
        occurred,
    );
    let next = proj.apply(&JobEvent::Transitioned(transitioned), occurred);
    assert_eq!(next.state, JobState::Intake);
    assert_eq!(next.version, 1);
}

// Keep `PolicyBaseline` import alive for future coverage of
// ReleaseCandidateStore tests in commit 4.
#[allow(dead_code)]
fn _policy_baseline_anchor() -> PolicyBaseline {
    PolicyBaseline::new(DistributionRef::new(
        DistributionFamily::new("debian").unwrap(),
        DistributionRelease::new("unstable").unwrap(),
    ))
}

// Compile-time check that the `IronMaintStore` facade is satisfied by
// `MockStore`. This catches accidental trait-drop regressions early.
#[allow(dead_code)]
fn _facade_satisfied(s: MockStore) -> impl IronMaintStore {
    s
}
