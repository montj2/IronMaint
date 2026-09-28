//! Phase 0B.5 C3 — Plan-aware `RunCheck { check_id }` +
//! per-gate `GateResult` evaluation (§15, §29, §41, §44).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, CheckId, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{EvidenceKind, EvidenceStatus, GateStatus};
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, Executor, ExecutorError, NullExecutor, RetryClass,
    ToolDefinitionRecord, ToolRegistry,
};
use ironmaint_runtime::{
    Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeErrorKind, RuntimeService,
    gate_stage_for,
};
use ironmaint_state::JobEvent as StateJobEvent;
use ironmaint_store::mock::MockStore;
use ironmaint_store::{
    CandidateStore, CheckStore, EventStore, EvidenceStore, GateStore, ProjectionStore,
};
use std::ffi::OsString;
use std::path::PathBuf;
use time::OffsetDateTime;
use url::Url;

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

fn make_candidate(job_id: JobId, fp_byte: u8) -> (SourceCandidate, CandidateFingerprint) {
    let url = Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit =
        GitObjectId::new(GitHashAlgorithm::Sha1, format!("{:x}", fp_byte).repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "f".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap());
    let candidate = SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now);
    (candidate.clone(), candidate.fingerprint().clone())
}

async fn create_job<E: ironmaint_executor::Executor + ?Sized>(
    svc: &RuntimeService<MockStore, E>,
    store: &Arc<MockStore>,
) -> JobId {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let side_effect = &result.side_effects[0];
    let rest = side_effect
        .strip_prefix("job:")
        .expect("prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    let id = JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("valid uuid"));

    let mut projection = store.get_projection(id).await.expect("projection");
    projection.state = JobState::SourceRevision;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("seed post-capture");
    id
}

async fn attach_candidate(
    store: &Arc<MockStore>,
    job_id: JobId,
    fp_byte: u8,
) -> (CandidateFingerprint, ironmaint_core::CandidateId) {
    let (candidate, fingerprint) = make_candidate(job_id, fp_byte);
    let cid = store
        .put_source_candidate(&candidate)
        .await
        .expect("put source");
    store
        .set_active_source_candidate(job_id, cid)
        .await
        .expect("active");
    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.active_candidate = Some(cid);
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("projection attach");
    (fingerprint, cid)
}

fn build_null_service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

fn make_registry_with(key_str: &str) -> Arc<ToolRegistry> {
    let mut reg = ToolRegistry::new();
    let record = ToolDefinitionRecord::new(
        ToolCapabilityKey::new(key_str).unwrap(),
        PathBuf::from("/usr/bin/true"),
        vec![OsString::from("--ignore-this")],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    );
    let boxed: Box<dyn ironmaint_executor::ToolDefinition> = Box::new(record);
    reg.register(boxed).expect("register tool");
    Arc::new(reg)
}

#[tokio::test]
async fn run_check_rejects_unknown_check_id() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_null_service(store.clone());
    let job_id = create_job(&svc, &store).await;
    let _ = attach_candidate(&store, job_id, 1).await;

    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id: CheckId::new(),
            retry_class: RetryClass::Safe,
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn run_check_rejects_candidate_mismatch() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_null_service(store.clone());
    let job_id = create_job(&svc, &store).await;
    let _ = attach_candidate(&store, job_id, 1).await;

    let other_fp = fingerprint_zero();
    let key = ToolCapabilityKey::new("synthetic.test.pass").unwrap();
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: other_fp,
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let check_id = ids[0];

    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect_err("must reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("candidate") && msg.contains("active"),
        "error must mention candidate/active mismatch, got: {msg}"
    );
}

#[tokio::test]
async fn run_check_rejects_pre_capture_state() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_null_service(store.clone());
    let job_id = create_job(&svc, &store).await;
    let (_fp, _cid) = attach_candidate(&store, job_id, 1).await;

    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.state = JobState::EventDetected;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("seed pre-capture");

    let key = ToolCapabilityKey::new("synthetic.test.pass").unwrap();
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: fingerprint_zero(),
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let check_id = ids[0];

    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn run_check_persists_evidence_and_gate_result_pass() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let executor = Arc::new(ScriptedExecutor::new_pass());
    let registry = make_registry_with("synthetic.test.pass");
    let svc = RuntimeService::new(store.clone(), fixed_clock(), executor, registry);
    let job_id = create_job(&svc, &store).await;
    let (fingerprint, _cid) = attach_candidate(&store, job_id, 1).await;

    let key = ToolCapabilityKey::new("synthetic.test.pass").unwrap();
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: fingerprint.clone(),
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let check_id = ids[0];

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("run check");

    let evidence = store
        .list_evidence_for_candidate(&fingerprint)
        .await
        .expect("list evidence");
    let latest = evidence
        .iter()
        .max_by_key(|e| e.observed_at)
        .expect("at least one evidence");
    assert_eq!(latest.status, EvidenceStatus::Pass);

    let check = store.get_check(check_id).await.expect("get check");
    let gate_result = store
        .get_gate_result(check.gate_id, &fingerprint)
        .await
        .expect("get gate result");
    assert_eq!(gate_result.status, GateStatus::Pass);
    assert_eq!(gate_result.gate_id, check.gate_id);
}

#[tokio::test]
async fn run_check_records_gate_result_fail_on_tool_failure() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let executor = Arc::new(ScriptedExecutor::new_fail());
    let registry = make_registry_with("synthetic.test.fail");
    let svc = RuntimeService::new(store.clone(), fixed_clock(), executor, registry);
    let job_id = create_job(&svc, &store).await;
    let (fingerprint, _cid) = attach_candidate(&store, job_id, 1).await;

    let key = ToolCapabilityKey::new("synthetic.test.fail").unwrap();
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: fingerprint.clone(),
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let check_id = ids[0];

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("run check; tool failure recorded as evidence");

    let check = store.get_check(check_id).await.expect("get check");
    let gate_result = store
        .get_gate_result(check.gate_id, &fingerprint)
        .await
        .expect("get gate result");
    assert_eq!(gate_result.status, GateStatus::Fail);
}

#[tokio::test]
async fn run_check_emits_tool_run_finished_audit_event() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let executor = Arc::new(ScriptedExecutor::new_pass());
    let registry = make_registry_with("synthetic.test.observe");
    let svc = RuntimeService::new(store.clone(), fixed_clock(), executor, registry);
    let job_id = create_job(&svc, &store).await;
    let (fingerprint, _cid) = attach_candidate(&store, job_id, 1).await;

    let key = ToolCapabilityKey::new("synthetic.test.observe").unwrap();
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: fingerprint.clone(),
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let check_id = ids[0];

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("run check");

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events");
    let has_tool_finished = events
        .iter()
        .any(|e| matches!(e.event, StateJobEvent::ToolRunFinished(_)));
    assert!(
        has_tool_finished,
        "expected one ToolRunFinished event, got: {events:?}"
    );
}

#[test]
fn gate_stage_for_maps_build_to_build_validation() {
    assert_eq!(
        gate_stage_for(&EvidenceKind::Build),
        Some(ironmaint_evidence::GateStage::BuildValidation)
    );
}

/// Local scripted executor: returns a fixed exit-code record
/// and is invoked at most once per test.
enum ScriptedOutcome {
    Pass,
    Fail,
}

struct ScriptedExecutor {
    outcome: Mutex<Option<ScriptedOutcome>>,
}

impl ScriptedExecutor {
    fn new_pass() -> Self {
        Self {
            outcome: Mutex::new(Some(ScriptedOutcome::Pass)),
        }
    }
    fn new_fail() -> Self {
        Self {
            outcome: Mutex::new(Some(ScriptedOutcome::Fail)),
        }
    }
}

impl Executor for ScriptedExecutor {
    async fn execute(
        &self,
        request: ironmaint_executor::ExecutionRequest,
    ) -> Result<ironmaint_executor::ExecutionRecord, ExecutorError> {
        let outcome = self
            .outcome
            .lock()
            .expect("lock")
            .take()
            .expect("scripted must run once");
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let exit_code = match outcome {
            ScriptedOutcome::Pass => 0,
            ScriptedOutcome::Fail => 1,
        };
        Ok(ironmaint_executor::ExecutionRecord {
            tool_key: request.tool_key.clone(),
            retry_class: RetryClass::Safe,
            started_at: now,
            finished_at: now,
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
            retries_exhausted: false,
            truncated: false,
        })
    }
}
