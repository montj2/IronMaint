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

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, CheckId, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{Evidence, EvidenceKind, EvidenceStatus, GateStatus};
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRequest, Executor, ExecutorError, ExecutorErrorKind,
    RetryClass, ToolDefinitionRecord, ToolRegistry,
};
use ironmaint_runtime::{Clock, FixedClock, RuntimeCommand, RuntimeService};
use ironmaint_state::{JobEvent, ToolOutcome};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{
    CandidateStore, CheckStore, EventStore, EvidenceStore, GateStore, ProjectionStore,
};
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

/// A [`ScriptedExecutor`] that answers by **tool key** rather than by call
/// order, and may be invoked any number of times.
///
/// [`ScriptedExecutor`] is deliberately one-shot: it `take()`s a single
/// response and panics on a second call, which is what makes every existing
/// test here able to assert that the executor ran *exactly once*. A test that
/// needs two checks in one gate cannot use it, and answering by call order
/// would make the test silently depend on the order the two `RunCheck`
/// commands happen to be issued in — so the lookup is keyed instead.
struct PerToolExecutor {
    responses: Mutex<HashMap<ToolCapabilityKey, ScriptedResponse>>,
}

impl PerToolExecutor {
    fn new(responses: Vec<(ToolCapabilityKey, ScriptedResponse)>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().collect()),
        })
    }
}

impl Executor for PerToolExecutor {
    async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ironmaint_executor::ExecutionRecord, ExecutorError> {
        let response = self
            .responses
            .lock()
            .expect("per-tool lock")
            .get(&request.tool_key)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "no scripted response for tool {:?}; the test must script every \
                     tool it runs",
                    request.tool_key
                )
            });
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

        // Constructed rather than executed: nothing spilled.
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    }
}

async fn seed_job_with_candidate<E: Executor>(
    store: &MockStore,
    svc: &RuntimeService<MockStore, E>,
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

    // §30 evidence freshness: `RunCheck { check_id }` requires
    // a post-capture state. Advance the projection's state
    // pointer to `SourceRevision` so the test body can call
    // `RunCheck` directly. The engine wiring added in C4 will
    // make this transition a real outcome of the state machine
    // rather than a test-only projection edit.
    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.state = JobState::SourceRevision;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("seed post-capture state");

    (job_id, fingerprint)
}

/// Materialise a single `CheckDefinition` for the given tool key
/// against the active candidate fingerprint. Returns the
/// `CheckId` (run via the runtime so per-job index + gate
/// definition are persisted atomically).
async fn materialize_check_for_tool<E: Executor>(
    svc: &RuntimeService<MockStore, E>,
    store: &Arc<MockStore>,
    job_id: JobId,
    fingerprint: CandidateFingerprint,
    tool_str: &str,
) -> CheckId {
    let key = ToolCapabilityKey::new(tool_str).expect("valid capability");
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: fingerprint,
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    let ids = store
        .list_checks_for_job(job_id)
        .await
        .expect("list checks");
    ids.into_iter()
        .next()
        .expect("at least one check from MaterializeChecks")
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

fn build_service<E: Executor>(
    store: Arc<MockStore>,
    executor: Arc<E>,
    registry_arc: Arc<ToolRegistry>,
) -> RuntimeService<MockStore, E> {
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

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.pass",
    )
    .await;

    let result = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");

    // Sequences 1 and 2: CreateJob's seed events (JobCreated, then the
    // Domain reference); 3: SetActiveCandidate; 4: ToolRunFinished.
    // (MaterializeChecks writes via put_check/put_gate_definition and does
    // not bump the event ledger.)
    assert_eq!(result.new_sequence, 4);

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

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.fail",
    )
    .await;

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
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

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.truncate",
    )
    .await;

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
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
async fn infrastructure_failure_records_infrastructure_error_evidence() {
    // An executor-level infrastructure failure (spawn error, worker
    // gone) is still a *check operation*, and §97 checkpoint 16
    // requires check operations to be persisted. §15 keeps it
    // distinct from a tool failure: the evidence carries
    // `InfrastructureError`, so gate aggregation blocks the gate
    // (§29) rather than failing it — an infrastructure problem is
    // not evidence that the package is broken.
    //
    // Until 0B.9 this propagated as a `RuntimeError` and wrote
    // nothing, which left the gate `NotEvaluated` forever and made
    // §15's "so gate evaluation can react accordingly" unreachable:
    // `ProcessExecutor` reports a spawn failure as an *error*, never
    // as a record with exit code 127, so the mapping arm below was
    // dead code in production.
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

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.infra_fail",
    )
    .await;

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("an infrastructure failure must still record a result");

    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(
        evidences.len(),
        1,
        "the failed check operation must be persisted exactly once"
    );
    assert_eq!(
        evidences[0].status,
        EvidenceStatus::InfrastructureError,
        "an infrastructure problem must not be recorded as a package failure"
    );
    // The producer is tagged so a synthesised record is
    // distinguishable from a real process exit.
    assert!(
        evidences[0]
            .producer
            .name
            .as_str()
            .ends_with("#executor-error"),
        "producer must be tagged as executor-derived, got {:?}",
        evidences[0].producer.name.as_str()
    );

    // And the gate must be Blocked, not Fail.
    let gate_id = store.get_check(check_id).await.expect("check").gate_id;
    let gate = store
        .get_gate_result(gate_id, &fingerprint)
        .await
        .expect("gate result");
    assert_eq!(
        gate.status,
        GateStatus::Blocked,
        "an InfrastructureError blocks the gate instead of failing it"
    );
}

#[tokio::test]
async fn a_gate_containing_an_unanswerable_check_does_not_report_a_verdict() {
    // The test above pins a gate with exactly ONE check. It passes, and it
    // has always passed, because `combine_status` folding a single
    // `Blocked` has nothing to outrank it with. The interesting case is
    // co-occurrence: one check in the gate genuinely failed, another
    // could not be answered at all.
    //
    // `GateStatus` documents the two as opposites — `Fail` is
    // "evaluation ran and found the gate violated", `Blocked` is
    // "evaluation couldn't complete". A gate that could not complete
    // has no verdict to report, so it must not report one. If it does,
    // `check.run` hands IronClaw a `Fail` for a package that may be
    // perfectly fine, the engine turns that into
    // `TransitionBlocker::FailedGate` ("repair this package") rather
    // than `IncompleteGate` ("this is not answerable"), and the
    // repair loop starts rewriting source to fix an outage.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.failing")))
        .expect("register");
    registry
        .register(Box::new(static_tool("synthetic.test.unreachable")))
        .expect("register");
    let registry_arc = Arc::new(registry);

    let failing_key = ToolCapabilityKey::new("synthetic.test.failing").expect("key");
    let unreachable_key = ToolCapabilityKey::new("synthetic.test.unreachable").expect("key");
    let executor = PerToolExecutor::new(vec![
        (
            failing_key.clone(),
            ScriptedResponse::Record(make_record(
                "synthetic.test.failing",
                1,
                "compile error".to_string(),
                false,
            )),
        ),
        (
            unreachable_key.clone(),
            ScriptedResponse::Error(ExecutorErrorKind::InfrastructureFailed),
        ),
    ]);
    let svc = build_service(store.clone(), executor, registry_arc);

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;

    // One check via the normal path, which also mints the gate.
    let failing_check = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.failing",
    )
    .await;

    // A second check on the SAME gate. `plan_to_materialised` mints a
    // fresh `GateId` per check, so two `MaterializeChecks` entries can
    // never share a gate — the only way to build a multi-check gate is to
    // write the `CheckDefinition` against the first one's `gate_id`.
    //
    // The two checks deliberately carry different `EvidenceKind`s.
    // `latest_evidence_for_check` selects by `ev.kind == check.evidence_kind`
    // and takes the latest by `observed_at`; give both checks the same
    // kind and each one reads the *same* evidence row, the fold sees one
    // status twice, and this test stops testing anything at all.
    let first = store.get_check(failing_check).await.expect("check");
    let second = ironmaint_store::CheckDefinition::new(
        job_id,
        fingerprint.clone(),
        first.gate_id,
        first.gate_stage,
        unreachable_key,
        EvidenceKind::Reproducibility,
        true,
    );
    store.put_check(&second).await.expect("put second check");

    for check_id in [failing_check, second.id] {
        svc.handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");
    }

    // Both evidence rows landed, one per kind — otherwise the assertion
    // below would be reading a gate that only one check ever answered.
    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(
        evidences.len(),
        2,
        "both checks must have produced evidence, or the aggregate is vacuous"
    );
    assert!(
        evidences.iter().any(|e| e.status == EvidenceStatus::Fail),
        "one check must have genuinely failed, or there is no verdict to suppress"
    );
    assert!(
        evidences
            .iter()
            .any(|e| e.status == EvidenceStatus::InfrastructureError),
        "one check must have been unanswerable, or there is nothing to test"
    );

    let gate = store
        .get_gate_result(first.gate_id, &fingerprint)
        .await
        .expect("gate result");
    assert_eq!(
        gate.evidence.len(),
        2,
        "the aggregate must carry both checks' evidence, or only one was folded"
    );
    assert_eq!(
        gate.status,
        GateStatus::Blocked,
        "a gate that could not be completed must not report `Fail`; \
         `Fail` tells the repair loop to rewrite a package that may be fine \
         (folded {} evidence row(s), status {:?})",
        gate.evidence.len(),
        gate.status
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

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.observe",
    )
    .await;

    let before = store
        .get_projection(job_id)
        .await
        .expect("projection")
        .version;
    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
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

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.timeout",
    )
    .await;

    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("a wall-clock timeout must still record a result");

    // §15: a timeout is a tool-side outcome, so it collapses to
    // `Fail` for evidence — but the audit event keeps
    // `ToolOutcome::Timeout`, which is asserted by
    // `timeout_outcome_propagates_to_tool_run_finished` below.
    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1, "timeout must be persisted");
    assert_eq!(
        evidences[0].status,
        EvidenceStatus::Fail,
        "a wall-clock timeout collapses to Fail for evidence purposes"
    );

    let gate_id = store.get_check(check_id).await.expect("check").gate_id;
    let gate = store
        .get_gate_result(gate_id, &fingerprint)
        .await
        .expect("gate result");
    assert_eq!(gate.status, GateStatus::Fail);
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

    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check_for_tool(
        &svc,
        &store,
        job_id,
        fingerprint.clone(),
        "synthetic.test.bad_path",
    )
    .await;

    // The point of this test is that a missing executable produces a
    // clean, recorded infrastructure failure — no unwrap, no
    // panic, and an audit trail the operator can read afterwards.
    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("a bad-path tool must surface as a recorded infra failure");

    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1, "bad-path run must be persisted");
    assert_eq!(evidences[0].status, EvidenceStatus::InfrastructureError);
}

// -----------------------------------------------------------------------------
// PR 1A.3 — `ResultNormalizer` is authoritative.
//
// Today, `handle_run_check` classifies every record via
// `ironmaint_executor::outcome_from_record`, which maps
// `exit_code == 0` to `Pass` and any other value to `Fail`.
// The `ResultNormalizer` trait has been declared on every
// `ToolDefinitionRecord` since 0B.4, but the runtime has
// never called it. PR 1A.3 changes that.
//
// The teeth-check is `result_normalizer_overrides_exit_code`:
// the scripted executor returns a record with `exit_code == 1`
// (which would map to `Fail` under the exit-code fallback) but
// the tool's `StructuredReportNormalizer` parses the stdout as
// `{"status":"pass"}` and returns `Pass`. A green run of this
// test is the proof that 1A.3's runtime wiring landed.
// -----------------------------------------------------------------------------

mod normalizer_teeth {
    use super::*;
    use ironmaint_executor::normalizer::{
        NormalizationError, NormalizedResult, Observation, ResultNormalizer,
    };

    /// A normalizer that classifies a record by parsing the
    /// child's stdout as `{"status": "pass"|"fail"}`. Mirrors
    /// the executor-side test normalizer in
    /// `tests/result_normalizer.rs`. Duplicated here rather
    /// than shared, because the two test files would otherwise
    /// form a cross-crate dependency cycle: the executor
    /// crate's tests do not depend on the runtime, and the
    /// runtime's tests should not depend on the executor's
    /// test scaffolding.
    #[derive(Debug)]
    pub(super) struct StructuredReportNormalizer;

    impl ResultNormalizer for StructuredReportNormalizer {
        fn normalize(
            &self,
            record: &ironmaint_executor::ExecutionRecord,
        ) -> Result<NormalizedResult, NormalizationError> {
            let v: serde_json::Value = serde_json::from_str(&record.stdout)
                .map_err(|e| NormalizationError::Malformed(format!("not valid JSON: {e}")))?;
            let status = v
                .get("status")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    NormalizationError::Malformed("missing string `status` field".to_string())
                })?;
            let observations = v
                .get("observations")
                .and_then(serde_json::Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|o| {
                            let kind = o.get("kind").and_then(serde_json::Value::as_str)?;
                            let message = o.get("message").and_then(serde_json::Value::as_str)?;
                            Some(Observation {
                                kind: kind.to_string(),
                                message: message.to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let evidence_status = match status {
                "pass" => EvidenceStatus::Pass,
                "fail" => EvidenceStatus::Fail,
                other => {
                    return Err(NormalizationError::Unclassifiable(format!(
                        "unknown status `{other}`"
                    )));
                }
            };
            Ok(NormalizedResult {
                evidence_status,
                output_truncated: record.truncated,
                observations,
                invalidations: Vec::new(),
            })
        }
    }

    fn tool_with_normalizer(
        key_str: &str,
        normalizer: Arc<dyn ResultNormalizer>,
    ) -> ToolDefinitionRecord {
        ToolDefinitionRecord::new(
            ToolCapabilityKey::new(key_str).unwrap(),
            PathBuf::from("/usr/bin/ironmaint-fixture"),
            vec![OsString::from("--validate")],
            ExecutionClass::Check,
            ExecutionLimits::default(),
        )
        .with_normalizer(normalizer)
    }

    #[tokio::test]
    async fn result_normalizer_overrides_exit_code() {
        let store: Arc<MockStore> = Arc::new(MockStore::new());
        let mut registry = ToolRegistry::new();
        let ck = "synthetic.test.normalizer_pass";
        registry
            .register(Box::new(tool_with_normalizer(
                ck,
                Arc::new(StructuredReportNormalizer) as Arc<dyn ResultNormalizer>,
            )))
            .expect("register");
        let registry_arc = Arc::new(registry);

        // exit_code == 1, stdout says pass. The exit-code
        // fallback would return Fail; the normalizer returns
        // Pass. PR 1A.3 makes the normalizer authoritative.
        let record = make_record(
            ck,
            1,
            r#"{"status":"pass","observations":[]}"#.to_string(),
            false,
        );
        let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
        let svc = build_service(store.clone(), executor, registry_arc);

        let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
        let check_id =
            materialize_check_for_tool(&svc, &store, job_id, fingerprint.clone(), ck).await;

        svc.handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");

        let evidences = store.list_evidence_for_job(job_id).await.expect("list");
        assert_eq!(evidences.len(), 1);
        let ev: &Evidence = &evidences[0];
        assert_eq!(
            ev.status,
            EvidenceStatus::Pass,
            "1A.3: the normalizer's evidence_status must drive the Evidence row, \
             not the exit-code fallback. Got {:?} from exit_code=1, stdout=pass.",
            ev.status
        );
    }

    #[tokio::test]
    async fn result_normalizer_normalization_error_becomes_evidence_fail() {
        // The runtime must not crash when a normalizer rejects
        // the body. A `NormalizationError` is a *tool*-level
        // outcome (the tool emitted something the runtime could
        // not classify), so the evidence status is `Fail` and
        // the gate sees the same outcome it would have seen if
        // the tool's own exit code had been non-zero.
        let store: Arc<MockStore> = Arc::new(MockStore::new());
        let mut registry = ToolRegistry::new();
        let ck = "synthetic.test.normalizer_malformed";
        registry
            .register(Box::new(tool_with_normalizer(
                ck,
                Arc::new(StructuredReportNormalizer) as Arc<dyn ResultNormalizer>,
            )))
            .expect("register");
        let registry_arc = Arc::new(registry);

        // exit_code == 0 (exit-code fallback would say Pass),
        // stdout is not valid JSON (normalizer rejects).
        let record = make_record(ck, 0, "not json at all".to_string(), false);
        let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
        let svc = build_service(store.clone(), executor, registry_arc);

        let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
        let check_id =
            materialize_check_for_tool(&svc, &store, job_id, fingerprint.clone(), ck).await;

        svc.handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check must not panic on a normalizer error");

        let evidences = store.list_evidence_for_job(job_id).await.expect("list");
        assert_eq!(evidences.len(), 1);
        assert_eq!(
            evidences[0].status,
            EvidenceStatus::Fail,
            "a NormalizationError must surface as EvidenceStatus::Fail, not as an error panic"
        );
    }

    #[tokio::test]
    async fn tool_without_normalizer_keeps_exit_code_fallback() {
        // Pinned regression: 1A.3 must not widen the
        // classification path. A tool that does not declare a
        // normalizer still uses `outcome_from_record` —
        // `exit_code == 0` is `Pass`, `exit_code != 0` is
        // `Fail`. This is the contract every existing
        // `synthetic.build.*` tool relies on.
        let store: Arc<MockStore> = Arc::new(MockStore::new());
        let mut registry = ToolRegistry::new();
        let ck = "synthetic.test.no_normalizer";
        registry
            .register(Box::new(static_tool(ck)))
            .expect("register");
        let registry_arc = Arc::new(registry);

        let record = make_record(ck, 1, "stdout: not classified".to_string(), false);
        let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
        let svc = build_service(store.clone(), executor, registry_arc);

        let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
        let check_id =
            materialize_check_for_tool(&svc, &store, job_id, fingerprint.clone(), ck).await;

        svc.handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");

        let evidences = store.list_evidence_for_job(job_id).await.expect("list");
        assert_eq!(evidences.len(), 1);
        assert_eq!(
            evidences[0].status,
            EvidenceStatus::Fail,
            "a tool without a normalizer must still use the exit-code fallback"
        );
    }

    #[tokio::test]
    async fn normalizer_succeeded_binds_report_artifact_to_evidence() {
        // PHASE-1.md §31 ("store it as a report artifact ...
        // bind the artifact to Evidence"): when the normalizer
        // accepts the record, the runtime writes the recorded
        // stdout to the artifact store and attaches a
        // `Report` `ArtifactRef` to the Evidence row. This is
        // the integration 1A.3 has to land — the status-only
        // check above proves the normalizer is called; this
        // one proves the artifact is on disk and bound.
        use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
        let tmp = tempfile::tempdir().expect("tempdir");
        let artifact_store = Arc::new(ArtifactStore::open(ArtifactRoot::new(
            tmp.path().to_path_buf(),
        )));

        let store: Arc<MockStore> = Arc::new(MockStore::new());
        let mut registry = ToolRegistry::new();
        let ck = "synthetic.test.normalizer_artifact";
        registry
            .register(Box::new(tool_with_normalizer(
                ck,
                Arc::new(StructuredReportNormalizer) as Arc<dyn ResultNormalizer>,
            )))
            .expect("register");
        let registry_arc = Arc::new(registry);

        // exit_code == 1 (would be Fail via the fallback),
        // stdout is a valid pass report, and the normalizer
        // accepts it. The integration is the normalizer
        // accepting + the artifact bound to the Evidence row.
        let stdout = r#"{"status":"pass","observations":[]}"#.to_string();
        let record = make_record(ck, 1, stdout.clone(), false);
        let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
        let svc = build_service(store.clone(), executor, registry_arc)
            .with_artifact_store(artifact_store.clone());

        let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
        let check_id =
            materialize_check_for_tool(&svc, &store, job_id, fingerprint.clone(), ck).await;

        svc.handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");

        let evidences = store.list_evidence_for_job(job_id).await.expect("list");
        assert_eq!(evidences.len(), 1);
        let ev: &Evidence = &evidences[0];
        assert_eq!(ev.status, EvidenceStatus::Pass);

        // Exactly one `Report` artifact on the row, with the
        // recorded stdout's bytes on disk under the artifact
        // store's sharded layout.
        let artifacts = &ev.artifacts;
        assert_eq!(
            artifacts.len(),
            1,
            "1A.3 binds exactly one `Report` artifact when the normalizer succeeds; got {artifacts:?}"
        );
        let art = &artifacts[0];
        assert_eq!(art.kind, ironmaint_evidence::ArtifactKind::Report);
        // The artifact's digest must round-trip through
        // `Digest::new` into a `Sha256Hex` (the artifacts
        // store's digest type) — that is the contract 1A.3
        // sets: a `Digest` is just a typed wrapper over
        // `(algorithm, hex)`. Then we re-fetch the bytes via
        // the artifact store to confirm the runtime's write
        // hit the same content-addressed path.
        let rec = artifact_store
            .put_bytes(stdout.as_bytes())
            .await
            .expect("put_bytes");
        let on_disk = artifact_store.get(&rec.digest).await.expect("get");
        assert_eq!(
            String::from_utf8(on_disk).expect("utf8"),
            stdout,
            "the bound artifact's digest must resolve to the recorded stdout verbatim"
        );
        // The bound artifact's digest must equal the
        // content-addressed digest of those bytes (the
        // runtime stored the same content, so the digest
        // matches by content-addressing).
        let bound_hex = art.digest.value.to_string();
        assert_eq!(
            bound_hex,
            rec.digest.as_str(),
            "the bound artifact's digest must equal the content-addressed digest of the recorded stdout"
        );
    }

    #[tokio::test]
    async fn normalizer_failure_does_not_bind_artifact() {
        // Pinned regression of the artifact-binding branch: a
        // `NormalizationError` produces no `Report` artifact on
        // the Evidence row (the normalizer rejected the body,
        // there is nothing well-formed to persist). The status
        // is still `Fail`, but the `artifacts` list is empty so
        // an operator can tell "tool emitted garbage" from
        // "tool emitted a structured fail report".
        use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
        let tmp = tempfile::tempdir().expect("tempdir");
        let artifact_store = Arc::new(ArtifactStore::open(ArtifactRoot::new(
            tmp.path().to_path_buf(),
        )));

        let store: Arc<MockStore> = Arc::new(MockStore::new());
        let mut registry = ToolRegistry::new();
        let ck = "synthetic.test.normalizer_malformed_no_artifact";
        registry
            .register(Box::new(tool_with_normalizer(
                ck,
                Arc::new(StructuredReportNormalizer) as Arc<dyn ResultNormalizer>,
            )))
            .expect("register");
        let registry_arc = Arc::new(registry);

        let record = make_record(ck, 0, "not json at all".to_string(), false);
        let executor = ScriptedExecutor::new(ScriptedResponse::Record(record));
        let svc = build_service(store.clone(), executor, registry_arc)
            .with_artifact_store(artifact_store.clone());

        let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
        let check_id =
            materialize_check_for_tool(&svc, &store, job_id, fingerprint.clone(), ck).await;

        svc.handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");

        let evidences = store.list_evidence_for_job(job_id).await.expect("list");
        assert_eq!(evidences.len(), 1);
        let ev: &Evidence = &evidences[0];
        assert_eq!(ev.status, EvidenceStatus::Fail);
        assert!(
            ev.artifacts.is_empty(),
            "a NormalizationError must not bind any artifact; got {:?}",
            ev.artifacts
        );
    }
}
