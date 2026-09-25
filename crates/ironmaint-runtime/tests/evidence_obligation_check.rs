//! End-to-end tests for the three `RuntimeService::handle_command`
//! handlers wired in 0B.18: `RecordCheckEvidence`,
//! `MarkObligationSatisfied`, and `RunCheck`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{EvidenceKind, EvidenceProducer, EvidenceStatus};
use ironmaint_executor::{
    ExecutionRecord, ExecutionRequest, Executor, ExecutorError, NullExecutor, RetryClass,
    ToolDefinition, ToolRegistry,
};
use ironmaint_policy::{Obligation, ObligationStatus, PolicyReference};
use ironmaint_runtime::{
    Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeErrorKind, RuntimeService,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EvidenceStore, ObligationStore};
use time::OffsetDateTime;
use url::Url;

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

/// Test executor that returns a fixed `ExecutionRecord`.
struct StaticExecutor {
    record: ExecutionRecord,
}

impl Executor for StaticExecutor {
    async fn execute(&self, _request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        Ok(self.record.clone())
    }
}

/// Tool registration that succeeds (exit_code 0) for "synthetic.test.pass"
/// and fails (exit_code 1) for "synthetic.test.fail".
struct StaticTool {
    key: ToolCapabilityKey,
}

impl ToolDefinition for StaticTool {
    fn key(&self) -> &ToolCapabilityKey {
        &self.key
    }
    fn normalizer(&self) -> Option<Box<dyn ironmaint_executor::ResultNormalizer>> {
        None
    }
}

/// Build a service backed by `NullExecutor` + an empty registry.
fn null_service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

/// Build a service backed by `StaticExecutor` + a registry
/// containing `synthetic.test.pass` (exit_code 0).
fn static_service(
    store: Arc<MockStore>,
    record: ExecutionRecord,
) -> (RuntimeService<MockStore, StaticExecutor>, Arc<ToolRegistry>) {
    let mut registry = ToolRegistry::new();
    let key = ToolCapabilityKey::new("synthetic.test.pass").unwrap();
    registry
        .register(Box::new(StaticTool { key }))
        .expect("register");
    let executor = StaticExecutor { record };
    let registry_arc = Arc::new(registry);
    let svc = RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(executor),
        registry_arc.clone(),
    );
    (svc, registry_arc)
}

async fn make_job<E: Executor + ?Sized>(
    store: &MockStore,
    svc: &RuntimeService<MockStore, E>,
) -> JobId {
    let r = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    parse_job_id(&r.side_effects[0], store).await
}

/// Parse a JobId out of a `job:<uuid> created by ...` side-effect
/// string. The MockStore assigns JobIds but the runtime constructs
/// them internally — so we read the projection back to discover
/// the actual id.
async fn parse_job_id(side_effect: &str, _store: &MockStore) -> JobId {
    let _ = side_effect;
    // The runtime mints a fresh JobId internally and persists the
    // projection. We can't list projections, but the side-effect
    // string carries the id we need to parse.
    let rest = side_effect
        .strip_prefix("job:")
        .expect("side-effect prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("valid uuid"))
}

async fn seed_candidate(
    store: &MockStore,
    job_id: JobId,
) -> (ironmaint_core::CandidateId, CandidateFingerprint) {
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap());
    let candidate = SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now);
    let fingerprint = candidate.fingerprint().clone();
    let id = store
        .put_source_candidate(&candidate)
        .await
        .expect("put_source_candidate");
    (id, fingerprint)
}

#[tokio::test]
async fn record_check_evidence_persists_evidence_record() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;

    // Activate the candidate so the projection has a fingerprint.
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let result = svc
        .handle_command(RuntimeCommand::RecordCheckEvidence {
            job_id,
            tool_key: "synthetic.test.pass".to_string(),
            evidence_kind: EvidenceKind::Build,
            status: EvidenceStatus::Pass,
            producer: "test-runner".to_string(),
        })
        .await
        .expect("record evidence");

    assert_eq!(result.new_sequence, 3); // create=1, set_active=2, evidence=3
    assert!(result.side_effects.is_empty());

    // Evidence list now contains the new evidence, bound to our
    // candidate's fingerprint.
    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1);
    let ev = &evidences[0];
    assert_eq!(ev.kind, EvidenceKind::Build);
    assert_eq!(ev.status, EvidenceStatus::Pass);
    assert_eq!(ev.candidate, fingerprint);
    assert_eq!(ev.producer, EvidenceProducer::new("test-runner"));
}

#[tokio::test]
async fn record_check_evidence_rejects_without_active_candidate() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;

    let err = svc
        .handle_command(RuntimeCommand::RecordCheckEvidence {
            job_id,
            tool_key: "synthetic.test.pass".to_string(),
            evidence_kind: EvidenceKind::Build,
            status: EvidenceStatus::Pass,
            producer: "test-runner".to_string(),
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn mark_obligation_satisfied_updates_obligation_status() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    // Seed an obligation directly into the store.
    let mut obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ironmaint_policy::ObligationStrength::Mandatory,
        ironmaint_policy::Applicability::Applicable,
        "lintian-clean",
    )
    .unwrap();
    obligation = obligation.with_status(ObligationStatus::NotEvaluated);
    let obligation_id = store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let result = svc
        .handle_command(RuntimeCommand::MarkObligationSatisfied {
            job_id,
            obligation_ref: "lintian-clean".to_string(),
        })
        .await
        .expect("mark obligation");
    assert!(!result.side_effects.is_empty());

    let updated = store.get_obligation(obligation_id).await.expect("get");
    assert_eq!(updated.status, ObligationStatus::Pass);
}

#[tokio::test]
async fn mark_obligation_satisfied_rejects_unknown_ref() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint,
    })
    .await
    .expect("set active");

    let err = svc
        .handle_command(RuntimeCommand::MarkObligationSatisfied {
            job_id,
            obligation_ref: "no-such-obligation".to_string(),
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn run_check_invokes_executor_and_records_evidence() {
    let store = Arc::new(MockStore::new());
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let record = ExecutionRecord {
        tool_key: ToolCapabilityKey::new("synthetic.test.pass").unwrap(),
        retry_class: RetryClass::Idempotent,
        started_at: now,
        finished_at: now + time::Duration::milliseconds(10),
        exit_code: 0,
        stdout: "OK".to_string(),
        stderr: String::new(),
        retries_exhausted: false,
    };
    let (svc, _registry) = static_service(store.clone(), record);

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint,
    })
    .await
    .expect("set active");

    let result = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            tool_key: "synthetic.test.pass".to_string(),
            retry_class: RetryClass::Idempotent,
        })
        .await
        .expect("run check");

    assert_eq!(result.new_sequence, 3);
    assert!(!result.side_effects.is_empty());

    // Evidence was recorded.
    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1);
}

#[tokio::test]
async fn run_check_rejects_unknown_tool() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint,
    })
    .await
    .expect("set active");

    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            tool_key: "unregistered.tool".to_string(),
            retry_class: RetryClass::Idempotent,
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}
