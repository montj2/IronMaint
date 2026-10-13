//! `check.run` and `operation.get` at the MCP wire boundary.
//!
//! Both tools shipped in 0B.6 as stubs: `check.run` echoed the
//! caller's own `tool_key` back with a hardcoded all-zero verdict,
//! and `operation.get` returned a sentinel `proposed` record for
//! *every* id. Neither could fail, and neither could report
//! anything that had actually happened.
//!
//! These tests drive the dispatcher exactly as a client would —
//! JSON in, JSON out, no store handles — and assert that the
//! answers are the durable ones.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, CheckId, DistributionFamily, DistributionRef, DistributionRelease, JobId,
    JobState, OperationId, PackageIdentity, PackageName, PackageRevision, PackageVersion,
    RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::EvidenceKind;
use ironmaint_executor::{
    ExecutionRecord, ExecutionRequest, Executor, ExecutorError, RetryClass, ToolRegistry,
};
use ironmaint_mcp::schema::McpToolName;
use ironmaint_mcp::{McpRuntime, dispatch};
use ironmaint_policy::{PrivilegedOperation, PrivilegedOperationKind};
use ironmaint_runtime::{RuntimeCommand, RuntimeService, SystemClock};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, CheckStore, OperationStore, ProjectionStore};
use time::OffsetDateTime;
use url::Url;

const TOOL: &str = "synthetic.build.validate";

/// Always-passes executor: the tests care about what the runtime
/// derives from a verdict, not about producing one.
#[derive(Clone)]
struct PassExecutor;

impl Executor for PassExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        Ok(ExecutionRecord {
            tool_key: request.tool_key.clone(),
            retry_class: RetryClass::Safe,
            started_at: now,
            finished_at: now,
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            retries_exhausted: false,
            truncated: false,

            // Constructed rather than executed: nothing spilled.
            artifacts: Vec::new(),
            artifacts_dropped: Vec::new(),
        })
    }
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").expect("debian"),
            DistributionRelease::new("unstable").expect("unstable"),
        ),
        PackageName::new("fixture-pkg").expect("name"),
    )
}

struct Fixture {
    mcp: McpRuntime<MockStore, PassExecutor>,
    store: Arc<MockStore>,
}

fn build() -> Fixture {
    let store = Arc::new(MockStore::new());
    let clock: Arc<dyn ironmaint_runtime::Clock> = Arc::new(SystemClock);
    let registry = Arc::new(registry_with_validate_tool());
    let service = Arc::new(RuntimeService::new(
        Arc::clone(&store),
        clock,
        Arc::new(PassExecutor),
        registry,
    ));
    Fixture {
        mcp: McpRuntime::new(service),
        store,
    }
}

/// A registry carrying exactly one tool definition for `TOOL`.
///
/// The binary path is deliberately nonexistent: `PassExecutor` never
/// spawns anything, but `RunCheck` still refuses an unregistered
/// capability — which is itself the point being tested, so the
/// registry has to name the tool the check was materialised for.
fn registry_with_validate_tool() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(ironmaint_executor::ToolDefinitionRecord::new(
            ToolCapabilityKey::new(TOOL).expect("key"),
            std::path::Path::new("/nonexistent/ironmaint-fixture-for-test"),
            vec![std::ffi::OsString::from("--validate")],
            ironmaint_executor::ExecutionClass::Check,
            ironmaint_executor::ExecutionLimits::default(),
        )))
        .expect("register");
    registry
}

async fn call(f: &Fixture, tool: &str, input: serde_json::Value) -> serde_json::Value {
    dispatch(f.mcp.clone(), &McpToolName(tool.to_string()), input)
        .await
        .unwrap_or_else(|e| panic!("{tool} failed: {e}"))
}

fn create_input() -> serde_json::Value {
    serde_json::json!({
        "orchestrator": { "kind": "ironclaw" },
        "package": {
            "distribution": { "family": "debian", "release": "sid" },
            "source_name": "fixture-pkg",
            "binary_names": [],
        },
    })
}

/// Create a job and materialise one check for `TOOL`, returning
/// both ids.
///
/// The check is materialised through `RuntimeService` rather than
/// a tool because 0B.9 does not add a `check.materialize` tool —
/// adapters are supposed to do this. Using the runtime handle here
/// is the test standing in for an adapter, not the MCP layer
/// reaching around the runtime.
async fn job_with_check(f: &Fixture) -> (JobId, CheckId) {
    let created = call(f, "job.create", create_input()).await;
    let job_id: JobId = serde_json::from_value(created["job_id"].clone()).expect("job id");

    let candidate = SourceCandidate::new(
        job_id,
        PackageRevision::new(package(), PackageVersion::new("1.0.0").expect("version")),
        RepositoryRef::new(
            VcsKind::Git,
            Url::parse("https://example.invalid/foo.git").unwrap(),
        )
        .expect("repository"),
        ironmaint_core::GitObjectId::new(ironmaint_core::GitHashAlgorithm::Sha1, "1".repeat(40))
            .expect("commit"),
        ironmaint_core::GitObjectId::new(ironmaint_core::GitHashAlgorithm::Sha1, "2".repeat(40))
            .expect("tree"),
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    let fingerprint: CandidateFingerprint = candidate.fingerprint().clone();
    f.store
        .put_source_candidate(&candidate)
        .await
        .expect("put candidate");

    f.mcp
        .service()
        .handle_command(RuntimeCommand::SetActiveCandidate {
            job_id,
            fingerprint: fingerprint.clone(),
        })
        .await
        .expect("set active");

    // `RunCheck` requires a post-capture state (§30).
    let mut projection = f.store.get_projection(job_id).await.expect("projection");
    projection.state = JobState::SourceRevision;
    f.store
        .put_projection(&projection, projection.version)
        .await
        .expect("seed state");

    f.mcp
        .service()
        .handle_command(RuntimeCommand::MaterializeChecks {
            job_id,
            candidate: fingerprint,
            planned: vec![(
                ToolCapabilityKey::new(TOOL).expect("key"),
                EvidenceKind::Build,
                true,
            )],
        })
        .await
        .expect("materialize");

    let check_id = f
        .store
        .list_checks_for_job(job_id)
        .await
        .expect("list checks")
        .into_iter()
        .next()
        .expect("a check");
    (job_id, check_id)
}

// ---------------------------------------------------------------------------
// check.run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_run_reports_the_recorded_evidence_and_gate_verdict() {
    // The 0B.6 stub returned `{evidence_status: "pass",
    // gate_status: "pass", exit_code: 0, ...}` unconditionally.
    // The stub could not distinguish success from failure, so this
    // asserts the real chain: run → evidence row → gate verdict,
    // all read back through the runtime's own query.
    let f = build();
    let (job_id, check_id) = job_with_check(&f).await;

    let out = call(
        &f,
        "check.run",
        serde_json::json!({ "job_id": job_id, "check_id": check_id }),
    )
    .await;

    assert_eq!(out["job_id"], serde_json::json!(job_id));
    assert_eq!(out["check_id"], serde_json::json!(check_id));
    assert_eq!(out["evidence_status"], serde_json::json!("pass"));
    assert_eq!(out["gate_status"], serde_json::json!("pass"));
    assert_eq!(out["truncated"], serde_json::json!(false));
    assert!(
        !out["evidence_id"].is_null(),
        "a real evidence id must be reported: {out}"
    );
}

#[tokio::test]
async fn check_run_output_carries_no_raw_executor_bytes() {
    // §92 / SKILL.md: the agent reasons about evidence and gate
    // verdicts, not stdout. `exit_code` is persisted nowhere, so a
    // tool that returned it would be reporting a value the store
    // cannot corroborate.
    let f = build();
    let (job_id, check_id) = job_with_check(&f).await;
    let out = call(
        &f,
        "check.run",
        serde_json::json!({ "job_id": job_id, "check_id": check_id }),
    )
    .await;

    for forbidden in ["exit_code", "stdout", "stderr"] {
        assert!(
            out.get(forbidden).is_none(),
            "{forbidden} must not be in the tool output: {out}"
        );
    }
}

#[tokio::test]
async fn check_run_rejects_an_unknown_check_id() {
    let f = build();
    let (job_id, _check_id) = job_with_check(&f).await;
    let err = dispatch(
        f.mcp.clone(),
        &McpToolName("check.run".to_string()),
        serde_json::json!({ "job_id": job_id, "check_id": CheckId::new() }),
    )
    .await
    .expect_err("unknown check must error");
    assert!(
        matches!(err, ironmaint_mcp::McpError::Runtime(_)),
        "expected a runtime error, got {err:?}"
    );
}

#[tokio::test]
async fn check_run_rejects_a_missing_check_id_field() {
    // The stub's `tool_key` field is gone; a client still sending
    // the old shape must get a clear input error rather than a
    // silently-wrong check being run.
    let f = build();
    let (job_id, _check_id) = job_with_check(&f).await;
    let err = dispatch(
        f.mcp.clone(),
        &McpToolName("check.run".to_string()),
        serde_json::json!({ "job_id": job_id, "tool_key": TOOL }),
    )
    .await
    .expect_err("stale tool_key input must be rejected");
    assert!(
        matches!(err, ironmaint_mcp::McpError::InvalidInput(_)),
        "expected an input error, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// operation.get
// ---------------------------------------------------------------------------

#[tokio::test]
async fn operation_get_round_trips_a_stored_operation() {
    let f = build();
    let (job_id, _check_id) = job_with_check(&f).await;
    let fingerprint = CandidateFingerprint::from_hex("a".repeat(64)).expect("fingerprint");

    let operation = PrivilegedOperation::proposed(
        PrivilegedOperationKind::CanonicalRepositoryPush,
        fingerprint,
    );
    let operation_id: OperationId = f
        .store
        .put_operation(&operation, job_id)
        .await
        .expect("put operation");

    let out = call(
        &f,
        "operation.get",
        serde_json::json!({ "operation_id": operation_id }),
    )
    .await;

    assert_eq!(out["operation"]["id"], serde_json::json!(operation_id));
    assert_eq!(
        out["operation"]["kind"],
        serde_json::json!("canonical_repository_push")
    );
    assert_eq!(
        out["operation"]["authorization"],
        serde_json::json!("proposed")
    );
}

#[tokio::test]
async fn operation_get_rejects_an_unknown_id() {
    // The 0B.6 stub answered every id with the same placeholder, so
    // this call could not fail. A caller could not tell a real
    // `Authorized` push from the placeholder — which is the one
    // distinction that matters for a side-effecting operation.
    let f = build();
    let (_job_id, _check_id) = job_with_check(&f).await;
    let err = dispatch(
        f.mcp.clone(),
        &McpToolName("operation.get".to_string()),
        serde_json::json!({ "operation_id": OperationId::new() }),
    )
    .await
    .expect_err("unknown operation must error");
    assert!(
        matches!(err, ironmaint_mcp::McpError::Runtime(_)),
        "expected a runtime error, got {err:?}"
    );
    assert!(err.to_string().contains("unknown operation"), "{err}");
}

// ---------------------------------------------------------------------------
// release.candidate.create
// ---------------------------------------------------------------------------

/// §101 steps 26-27. The snapshot has to be reachable over the wire,
/// and it has to name the candidate the job was actually validated
/// against rather than the first one it ever captured.
#[tokio::test]
async fn release_candidate_create_returns_the_snapshot_of_the_active_candidate() {
    let f = build();
    let (job_id, _check_id) = job_with_check(&f).await;
    // C1 is whatever `job_with_check` activated. Read through the
    // store rather than a tool: `job.get` reports the projection's
    // `active_candidate` as a `CandidateId`, not a fingerprint, and
    // no tool exposes a candidate by id — §53 lists
    // `ironmaint_candidate_get` in the read-only set and 0B never
    // implemented it. Worth knowing and worth its own tool later;
    // not this test's business.
    let c1: CandidateFingerprint = f
        .store
        .get_source_candidate(
            f.store
                .active_source_candidate(job_id)
                .await
                .expect("active candidate")
                .expect("an active candidate"),
        )
        .await
        .expect("the active candidate row")
        .fingerprint()
        .clone();

    // Supersede it, so a snapshot that ignored the active candidate
    // would return C1's.
    let c2 = SourceCandidate::new(
        job_id,
        PackageRevision::new(package(), PackageVersion::new("1.0.1").expect("version")),
        RepositoryRef::new(
            VcsKind::Git,
            Url::parse("https://example.invalid/foo.git").unwrap(),
        )
        .expect("repository"),
        ironmaint_core::GitObjectId::new(ironmaint_core::GitHashAlgorithm::Sha1, "1".repeat(40))
            .expect("commit"),
        ironmaint_core::GitObjectId::new(ironmaint_core::GitHashAlgorithm::Sha1, "3".repeat(40))
            .expect("tree"),
        OffsetDateTime::from_unix_timestamp(1_700_000_001).unwrap(),
    );
    let c2_fingerprint = c2.fingerprint().clone();
    f.store
        .put_source_candidate(&c2)
        .await
        .expect("put the second candidate");
    f.mcp
        .service()
        .handle_command(RuntimeCommand::SetActiveCandidate {
            job_id,
            fingerprint: c2_fingerprint.clone(),
        })
        .await
        .expect("activate the second candidate");

    let out = call(
        &f,
        "release.candidate.create",
        serde_json::json!({ "job_id": job_id }),
    )
    .await;
    let release: ironmaint_policy::ReleaseCandidate =
        serde_json::from_value(out["release_candidate"].clone())
            .expect("the snapshot deserializes");
    assert_eq!(release.source, c2_fingerprint, "§101 step 27");
    assert_ne!(release.source, c1, "and not the superseded candidate");
    assert!(
        release.gate_ids.is_empty(),
        "C2 was never materialised with a check, so the snapshot has no \
         gates — and C1's gate did not leak in. The gates are filtered by \
         fingerprint, not by job, which is what §101 step 27 is about: \
         {:?}",
        release.gate_ids
    );

    // Idempotent on the wire, not just at the command: a second call
    // is a no-op a client can make without consequence.
    let again = call(
        &f,
        "release.candidate.create",
        serde_json::json!({ "job_id": job_id }),
    )
    .await;
    assert_eq!(
        again["release_candidate"]["id"], out["release_candidate"]["id"],
        "calling twice returns the same snapshot, not a second one"
    );
    assert_eq!(
        f.store
            .list_release_candidates_for_job(job_id)
            .await
            .expect("list snapshots")
            .len(),
        1
    );
}

#[tokio::test]
async fn release_candidate_create_names_why_there_is_nothing_to_snapshot() {
    let f = build();
    let created = call(&f, "job.create", create_input()).await;
    let job_id: JobId = serde_json::from_value(created["job_id"].clone()).expect("job id");

    // No active candidate: mid-workflow, not a failure.
    let err = dispatch(
        f.mcp.clone(),
        &McpToolName("release.candidate.create".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect_err("a job with no candidate has no release candidate");
    assert!(
        err.to_string().contains("no active candidate"),
        "the error must say which of the two reasons it is, or a client \
         cannot tell 'keep working' from 'assemble the snapshot': {err}"
    );
}
