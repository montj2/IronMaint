//! Smoke tests for the SQLite backend.
//!
//! These exercise every sub-trait via `SqliteStore::open_in_memory`.
//! The in-memory path skips migrations, so foreign-key enforcement
//! runs against an empty schema. This catches obvious integration
//! errors in the trait impls; the production `open` (with
//! migrations applied) is exercised by the daemon in commit 14.

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
    SourceCandidate, VcsKind,
};
use ironmaint_evidence::{
    Evidence, EvidenceKind, EvidenceProducer, EvidenceScope, EvidenceStatus, GateDefinition,
    GateRequirement, GateResult, GateStage, RequiredEvidenceStatus,
};
use ironmaint_policy::{
    Applicability, AuthorizationState, Obligation, ObligationStrength, PrivilegedOperation,
    PrivilegedOperationKind,
};
use ironmaint_state::{JobEvent, StateTransitioned, Transition};
use ironmaint_store::mock::MockStore;
use ironmaint_store::workspace::WorkspaceState;
use ironmaint_store::{
    CandidateStore, EventStore, EvidenceStore, GateStore, IronMaintStore, ObligationStore,
    OperationStore, ProjectionStore, StoreErrorKind, WorkspaceMetadataStore,
};
use ironmaint_store_sqlite::SqliteStore;
use time::macros::datetime;
use url::Url;

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

fn transitioned_event_for(jid: JobId) -> StateTransitioned {
    StateTransitioned::new(
        Transition {
            from: JobState::EventDetected,
            to: JobState::Intake,
            rule_index: 0,
        },
        JobProjection {
            job: MaintenanceJob::new(
                jid,
                package(),
                MaintenanceEventId::new(),
                datetime!(2026-01-01 00:00:00 UTC),
            ),
            state: JobState::Intake,
            active_candidate: None,
            version: 1,
            updated_at: datetime!(2026-01-02 00:00:00 UTC),
        },
        datetime!(2026-01-02 00:00:00 UTC),
    )
}

/// Create the parent projection row so foreign-key constraints
/// accept subsequent event/candidate/evidence/operation writes.
async fn create_projection(s: &SqliteStore, jid: JobId) {
    let proj = initial_projection(JobState::EventDetected, 0);
    // The job's id is inside the projection; ensure it matches.
    let proj = JobProjection {
        job: MaintenanceJob::new(
            jid,
            package(),
            MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        ),
        ..proj
    };
    s.put_projection(&proj, 0).await.unwrap();
}

#[tokio::test]
async fn sqlite_satisfies_the_facade() {
    fn _assert_facade<T: IronMaintStore + ?Sized>(_: &T) {}
    let s = SqliteStore::open_in_memory().await.unwrap();
    _assert_facade(&s);
}

#[tokio::test]
async fn event_store_round_trip() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
    let env = ironmaint_store::EventEnvelope::new(
        MaintenanceEventId::new(),
        jid,
        1,
        datetime!(2026-01-02 00:00:00 UTC),
        JobEvent::Transitioned(transitioned_event_for(jid)),
    );
    s.append_event(&env).await.unwrap();
    let got = s.get_event(jid, 1).await.unwrap();
    assert_eq!(got, env);
    assert_eq!(s.next_sequence(jid).await.unwrap(), 2);
}

#[tokio::test]
async fn candidate_store_round_trip() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
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
}

#[tokio::test]
async fn evidence_store_round_trip() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
    let ev = Evidence::new(
        fingerprint(),
        EvidenceKind::Build,
        EvidenceStatus::Pass,
        EvidenceProducer::new("ironmaint-fixture"),
        EvidenceScope::Job(jid),
        datetime!(2026-01-02 00:00:00 UTC),
    );
    let id = s.put_evidence(&ev, jid).await.unwrap();
    let got = s.get_evidence(id).await.unwrap();
    assert_eq!(got, ev);
    let list = s.list_evidence_for_job(jid).await.unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn gate_store_round_trip() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
    let gate = GateDefinition::new(
        fingerprint(),
        GateStage::BuildValidation,
        GateRequirement::new(EvidenceKind::Build, RequiredEvidenceStatus::Pass),
        true,
    );
    let id = s.put_gate_definition(&gate, jid).await.unwrap();
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
    let got_r = s.get_gate_result(id, &gate.candidate).await.unwrap();
    assert_eq!(got_r, result);
}

#[tokio::test]
async fn obligation_store_round_trip() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
    let ob = Obligation::new(
        fingerprint(),
        ironmaint_policy::PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "lintian passes",
    )
    .expect("static literal fits within REQUIREMENT_MAX");
    let id = s.put_obligation(&ob, jid).await.unwrap();
    let got = s.get_obligation(id).await.unwrap();
    assert_eq!(got, ob);
}

#[tokio::test]
async fn operation_store_round_trip() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
    let op = PrivilegedOperation::proposed(
        PrivilegedOperationKind::CanonicalRepositoryPush,
        fingerprint(),
    );
    let id = s.put_operation(&op, jid).await.unwrap();
    let mut executing = op.clone();
    executing.authorization = AuthorizationState::Executing;
    s.update_operation(id, &executing).await.unwrap();
    let executing_list = s.list_executing_operations().await.unwrap();
    assert_eq!(executing_list, vec![id]);
}

#[tokio::test]
async fn workspace_metadata_round_trip() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
    let handle = "ws-1".to_string();
    let st = WorkspaceState {
        handle: handle.clone(),
        job_id: jid,
        revision: 0,
        base_candidate: None,
        dirty: false,
        created_at: time::OffsetDateTime::now_utc(),
        updated_at: time::OffsetDateTime::now_utc(),
    };
    s.put_workspace_state(&st, 0).await.unwrap();
    let got = s.get_workspace_state(&handle).await.unwrap();
    assert_eq!(got, st);
}

#[tokio::test]
async fn projection_store_round_trip_with_cas() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let jid = job_id();
    create_projection(&s, jid).await;
    let env = ironmaint_store::EventEnvelope::new(
        MaintenanceEventId::new(),
        jid,
        1,
        datetime!(2026-01-02 00:00:00 UTC),
        JobEvent::Transitioned(transitioned_event_for(jid)),
    );
    s.append_event(&env).await.unwrap();
    let rebuilt = s.rebuild_projection(jid).await.unwrap();
    assert_eq!(rebuilt.state, JobState::Intake);
    s.put_projection(&rebuilt, 0).await.unwrap();
    let got = s.get_projection(jid).await.unwrap();
    assert_eq!(got.state, JobState::Intake);
    let mut stale = got.clone();
    stale.version = 99;
    let err = s.put_projection(&stale, 0).await.unwrap_err();
    assert_eq!(*err.kind(), StoreErrorKind::Conflict);
}

#[tokio::test]
async fn sqlite_and_mock_agree_on_shape() {
    // Both backends must satisfy the same facade and expose the
    // same sub-trait surface; this catches accidental drift
    // between the in-memory test fixture and the SQLite impl.
    fn assert_facade<T: IronMaintStore + ?Sized>(_: &T) {}
    assert_facade(&SqliteStore::open_in_memory().await.unwrap());
    assert_facade(&MockStore::new());
}
