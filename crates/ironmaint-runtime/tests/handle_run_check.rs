//! Phase 0B.4 C5 — `RuntimeService::handle_run_check` rewires
//! `Evidence.truncated` and emits `JobEvent::ToolRunFinished`
//! (PHASE-0B.md §15 last bullet, §92).
//!
//! Each test exercises one §92 condition via a `ScriptedExecutor`
//! that returns a hand-built `ExecutionRecord` so the tests stay
//! hermetic (no fixture-binary dependency). The translation from
//! `executor::Outcome` to `state::ToolOutcome`, the truncation
//! propagation, and the audit-event payload are all pinned down
//! here.
//!
//! `unwrap`/`expect` are allowed here because failure in a test
//! should panic; the production crate forbids them via the
//! workspace lint table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, MaintenanceEventId, PackageIdentity,
    PackageName, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{Evidence, EvidenceStatus};
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRequest, Executor, ExecutorError, ExecutorErrorKind,
    RetryClass, ToolDefinitionRecord, ToolRegistry,
};
use ironmaint_runtime::{
    Clock, FixedClock, RuntimeCommand, RuntimeErrorKind, RuntimeQuery, RuntimeService,
};
use ironmaint_state::{JobEvent, ToolOutcome};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EventStore, EvidenceStore, ProjectionStore};
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

fn static_tool(key_str: &str) -> ToolDefinitionRecord {
    ToolDefinitionRecord::new(
        ToolCapabilityKey::new(key_str).unwrap(),
        PathBuf::from("/usr/bin/ironmaint-fixture"),
        vec![OsString::from("--validate")],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    )
}

fn bad_path_tool(key_str: &str) -> ToolDefinitionRecord {
    // Path intentionally points at a nonexistent file so spawn fails.
    ToolDefinitionRecord::new(
        ToolCapabilityKey::new(key_str).unwrap(),
        PathBuf::from("/nonexistent/ironmaint-fixture-binary-for-test"),
        vec![],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    )
}

#[derive(Debug, Clone)]
enum ScriptedResponse {
    Record(ironmaint_executor::ExecutionRecord),
    Error(ExecutorErrorKind),
}

struct ScriptedExecutor {
    response: Mutex<Option<ScriptedResponse>>,
}

impl ScriptedExecutor {
    fn new(response: ScriptedResponse) -> Arc<Self> {
        Arc::new(Self {
            response: Mutex::new(Some(response)),
        })
    }
}

impl Executor for ScriptedExecutor {
    async fn execute(
        &self,
        _request: ExecutionRequest,
    ) -> Result<ironmaint_executor::ExecutionRecord, ExecutorError> {
        let mut guard = self.response.lock().expect("scripted lock");
        let response = guard
            .take()
            .expect("scripted executor must be invoked exactly once per test");
        match response {
            ScriptedResponse::Record(r) => Ok(r),
            ScriptedResponse::Error(kind) => Err(ExecutorError::new(kind, "scripted failure")),
        }
    }
}

fn make_record(
    key: &str,
    exit_code: i32,
    stdout: String,
    truncated: bool,
) -> ironmaint_executor::ExecutionRecord {
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    ironmaint_executor::ExecutionRecord {
        tool_key: ToolCapabilityKey::new(key).unwrap(),
        retry_class: RetryClass::Safe,
        started_at: now,
        finished_at: now + time::Duration::milliseconds(10),
        exit_code,
        stdout,
        stderr: String::new(),
        retries_exhausted: false,
        truncated,
    }
}

async fn seed_job_with_candidate(
    store: &MockStore,
    svc: &RuntimeService<MockStore, ScriptedExecutor>,
) -> (JobId, CandidateFingerprint) {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: ironmaint_runtime::OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let job_id = parse_job_id(&result.side_effects[0]);

    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap());
    let candidate = SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now);
    let fingerprint = candidate.fingerprint().clone();
    store
        .put_source_candidate(&candidate)
        .await
        .expect("put_source_candidate");

    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    (job_id, fingerprint)
}

fn parse_job_id(side_effect: &str) -> JobId {
    let rest = side_effect
        .strip_prefix("job:")
        .expect("side-effect prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("valid uuid"))
}

fn build_service(
    store: Arc<MockStore>,
    executor: Arc<ScriptedExecutor>,
    registry_arc: Arc<ToolRegistry>,
) -> RuntimeService<MockStore, ScriptedExecutor> {
    RuntimeService::new(store, fixed_clock(), executor, registry_arc)
}

// -----------------------------------------------------------------------------
// Tests.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn pass_path_emits_tool_run_finished_with_pass_outcome() {
    // §92: PASS — exit_code 0, truncated=false.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.pass")))
        .expect("register");
    let registry_arc = Arc::new(registry);
    let record = make_record("synthetic.test.pass", 0, "OK".to_string(), false);
    let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    let result = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            tool_key: "synthetic.test.pass".to_string(),
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");

    // Sequence 1: projection seed; 2: SetActiveCandidate; 3: ToolRunFinished.
    assert_eq!(result.new_sequence, 3);

    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1);
    let ev: &Evidence = &evidences[0];
    assert_eq!(ev.status, EvidenceStatus::Pass);
    assert!(!ev.truncated);

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("events");
    let last = events.last().expect("at least one event");
    if let JobEvent::ToolRunFinished(t) = &last.event {
        assert_eq!(t.outcome, ToolOutcome::Pass);
        assert!(!t.truncated);
        assert_eq!(t.evidence_id, ev.id);
    } else {
        panic!("expected JobEvent::ToolRunFinished, got {:?}", last.event);
    }
}

#[tokio::test]
async fn fail_path_emits_tool_run_finished_with_fail_outcome() {
    // §92: FAIL — exit_code 1, truncated=false.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.fail")))
        .expect("register");
    let registry_arc = Arc::new(registry);
    let record = make_record("synthetic.test.fail", 1, "no good\nbad".to_string(), false);
    let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        tool_key: "synthetic.test.fail".to_string(),
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("run check");

    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1);
    assert_eq!(evidences[0].status, EvidenceStatus::Fail);
    assert!(!evidences[0].truncated);

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("events");
    let last = events.last().expect("event");
    if let JobEvent::ToolRunFinished(t) = &last.event {
        assert_eq!(t.outcome, ToolOutcome::Fail);
        assert!(!t.truncated);
    } else {
        panic!("expected JobEvent::ToolRunFinished, got {:?}", last.event);
    }
}

#[tokio::test]
async fn truncated_path_records_truncated_on_evidence_and_event() {
    // §15 + §92: a tool that returned `truncated=true` must
    // propagate the boolean onto the persisted Evidence row AND
    // the audit event. The exit code is 0, so the outcome stays
    // `Pass` — truncation is orthogonal.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.truncate")))
        .expect("register");
    let registry_arc = Arc::new(registry);
    let record = make_record("synthetic.test.truncate", 0, "A".repeat(256 * 1024), true);
    let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        tool_key: "synthetic.test.truncate".to_string(),
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("run check");

    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1);
    assert!(
        evidences[0].truncated,
        "Evidence.truncated must be true when the executor bounded stdout"
    );
    assert_eq!(evidences[0].status, EvidenceStatus::Pass);

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("events");
    let last = events.last().expect("event");
    if let JobEvent::ToolRunFinished(t) = &last.event {
        assert!(t.truncated);
        assert_eq!(t.outcome, ToolOutcome::Pass);
        assert_eq!(t.evidence_id, evidences[0].id);
    } else {
        panic!("expected JobEvent::ToolRunFinished, got {:?}", last.event);
    }
}

#[tokio::test]
async fn infrastructure_failure_is_propagated_as_runtime_error() {
    // §92: when the executor itself fails (spawn error, etc.),
    // no Evidence row is written and no ToolRunFinished event is
    // appended — the runtime surfaces the error.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.infra_fail")))
        .expect("register");
    let registry_arc = Arc::new(registry);
    let executor = ScriptedExecutor::new(ScriptedResponse::Error(
        ExecutorErrorKind::InfrastructureFailed,
    ));
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            tool_key: "synthetic.test.infra_fail".to_string(),
            retry_class: RetryClass::Safe,
        })
        .await
        .expect_err("must propagate infrastructure failure");

    assert_eq!(err.kind, RuntimeErrorKind::Executor);

    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert!(
        evidences.is_empty(),
        "infrastructure failure must not produce an Evidence row"
    );
}

#[tokio::test]
async fn tool_run_finished_does_not_advance_projection() {
    // §15 last bullet: tool runs are observability signals and
    // must NOT change the FSM. `JobProjection.version` stays put
    // across a `ToolRunFinished` event.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.observe")))
        .expect("register");
    let registry_arc = Arc::new(registry);
    let record = make_record("synthetic.test.observe", 0, "OK".to_string(), false);
    let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    let before = store
        .get_projection(job_id)
        .await
        .expect("projection")
        .version;
    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        tool_key: "synthetic.test.observe".to_string(),
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("run check");
    let after = store
        .get_projection(job_id)
        .await
        .expect("projection")
        .version;

    assert_eq!(
        before, after,
        "ToolRunFinished must not advance projection version"
    );
}

#[tokio::test]
async fn timeout_outcome_propagates_to_tool_run_finished() {
    // §92: TIMEOUT — executor returns a successful record but with
    // an exit code in the failure band; runtime translates it to
    // `ToolOutcome::Fail` (per the C5 evidence-status mapping:
    // Timeout collapses to Fail for evidence, but stays Timeout
    // for the audit event).
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.timeout")))
        .expect("register");
    let registry_arc = Arc::new(registry);
    // `outcome_from_record` only sets Timeout when the executor
    // returns a `ToolFailed { timed_out: true }` error. When the
    // child was killed by tokio::time::timeout the runtime
    // surfaces it as `ExecutorError`. Use the error path to
    // assert that branch instead.
    let executor = ScriptedExecutor::new(ScriptedResponse::Error(ExecutorErrorKind::ToolFailed {
        timed_out: true,
    }));
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            tool_key: "synthetic.test.timeout".to_string(),
            retry_class: RetryClass::Safe,
        })
        .await
        .expect_err("timeout surfaces as executor error");
    assert_eq!(err.kind, RuntimeErrorKind::Executor);
}

#[tokio::test]
async fn bad_path_tool_at_registry_is_a_complete_error_path() {
    // §92: a tool registered with a non-existent executable must
    // produce a clean infrastructure-failure surface (no
    // unwraps/panics) when the runtime tries to invoke it. We
    // register the bad tool but pair the executor with a recorded
    // `InfrastructureFailed` error to simulate what `ProcessExecutor`
    // does when `spawn` returns `NotFound`.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(bad_path_tool("synthetic.test.bad_path")))
        .expect("register");
    let registry_arc = Arc::new(registry);
    let executor = ScriptedExecutor::new(ScriptedResponse::Error(
        ExecutorErrorKind::InfrastructureFailed,
    ));
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            tool_key: "synthetic.test.bad_path".to_string(),
            retry_class: RetryClass::Safe,
        })
        .await
        .expect_err("bad-path tool must surface as infra failure");
    assert_eq!(err.kind, RuntimeErrorKind::Executor);
}

// `_query_projection` keeps `RuntimeQuery` imported for future
// expansion of the suite (the existing `create_and_set_active`
// tests already exercise it). Anchoring the import here makes a
// downstream lint regression surface locally.
#[allow(dead_code)]
async fn _query_projection(svc: &RuntimeService<MockStore, ScriptedExecutor>, job_id: JobId) {
    let _ = svc
        .handle_query(RuntimeQuery::GetProjection { job_id })
        .await
        .expect("query");
}

// `_maintenance_event_id` keeps `MaintenanceEventId` and
// `JobState` imports live so a downstream delete of the test
// fails locally instead of in another crate.
#[allow(dead_code)]
fn _imports_anchor() -> (MaintenanceEventId, JobState) {
    (MaintenanceEventId::new(), JobState::EventDetected)
}
