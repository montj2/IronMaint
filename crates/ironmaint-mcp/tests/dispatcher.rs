//! Automated MCP client walks the fixture scenario.
//!
//! §94 exit checkpoint: "An automated MCP client completes
//! the fixture scenario without direct store access."
//!
//! The test composes an MCP dispatcher backed by the full
//! runtime service (with `MockStore`, a scripted
//! always-passes `Executor`, and `SystemClock`), then issues
//! the same sequence of tool calls a real MCP client would
//! make — never touching `MockStore` directly. The test
//! asserts that the dispatcher round-trips every tool with a
//! sensible JSON shape and that the runtime service retains
//! the job_id it minted in the create response.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//!
//! §94 exit checkpoint: "An automated MCP client completes
//! the fixture scenario without direct store access."
//!
//! The test composes an MCP dispatcher backed by the full
//! runtime service (with `MockStore`, a scripted
//! always-passes `Executor`, and `SystemClock`), then issues
//! the same sequence of tool calls a real MCP client would
//! make — never touching `MockStore` directly. The test
//! asserts that the dispatcher round-trips every tool with a
//! sensible JSON shape and that the runtime service retains
//! the job_id it minted in the create response.

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, JobId, PackageIdentity, PackageName,
};
use ironmaint_executor::{
    ExecutionRecord, ExecutionRequest, Executor, ExecutorError, NullExecutor, RetryClass,
    ToolRegistry,
};
use ironmaint_mcp::schema::McpToolName;
use ironmaint_mcp::{McpRuntime, dispatch};
use ironmaint_runtime::{RuntimeService, SystemClock};
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::WorkspaceManager;
use time::OffsetDateTime;

/// Always-passes scripted executor — used so any
/// `RunCheck` tool path records `Pass` evidence without
/// spawning a subprocess.
#[derive(Clone)]
struct PassExecutor;

impl Executor for PassExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .unwrap_or_else(|_| OffsetDateTime::now_utc());
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
        })
    }
}

fn build_runtime() -> RuntimeService<MockStore, PassExecutor> {
    let store = Arc::new(MockStore::new());
    let clock: Arc<dyn ironmaint_runtime::Clock> = Arc::new(SystemClock);
    let executor = Arc::new(PassExecutor);
    let registry = Arc::new(ToolRegistry::new());
    RuntimeService::new(store, clock, executor, registry)
}

/// Build a dispatcher whose workspace tools are live, over a real
/// git-backed working tree rooted in a throwaway directory.
///
/// The store handle is shared: the runtime owns `Arc<MockStore>`
/// and the `WorkspaceManager` borrows the same `Arc`, which is how
/// a daemon is wired (§98.6 — no second connection).
fn build_runtime_with_workspace(root: &std::path::Path) -> McpRuntime<MockStore, PassExecutor> {
    let store = Arc::new(MockStore::new());
    let clock: Arc<dyn ironmaint_runtime::Clock> = Arc::new(SystemClock);
    let executor = Arc::new(PassExecutor);
    let registry = Arc::new(ToolRegistry::new());
    let service = Arc::new(RuntimeService::new(
        Arc::clone(&store),
        clock,
        executor,
        registry,
    ));
    let workspace = Arc::new(WorkspaceManager::new(root.to_path_buf(), store));
    McpRuntime::new(service).with_workspace(workspace)
}

fn debian_foo() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").expect("debian"),
            DistributionRelease::new("unstable").expect("unstable"),
        ),
        PackageName::new("fixture-pkg").expect("fixture-pkg"),
    )
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

#[tokio::test]
async fn dispatcher_round_trips_create_get_next_actions() {
    let mcp = McpRuntime::new(Arc::new(build_runtime()));

    let create_out = dispatch(
        mcp.clone(),
        &McpToolName("job.create".to_string()),
        create_input(),
    )
    .await
    .expect("create must succeed");
    let job_id: JobId =
        serde_json::from_value(create_out.get("job_id").cloned().expect("job_id present"))
            .expect("job_id deserializable");
    assert!(!job_id.as_uuid().is_nil());

    let get_out = dispatch(
        mcp.clone(),
        &McpToolName("job.get".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("get must succeed");
    let state = get_out
        .get("projection")
        .and_then(|v| v.get("state"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .expect("projection.state present");
    assert_eq!(state, "event_detected");

    let next_out = dispatch(
        mcp.clone(),
        &McpToolName("job.next_actions".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("next_actions must succeed");
    assert!(
        next_out.get("actions").is_some(),
        "actions field present: {next_out}"
    );
}

#[tokio::test]
async fn dispatcher_round_trips_reconcile_outcome() {
    let mcp = McpRuntime::new(Arc::new(build_runtime()));

    let create_out = dispatch(
        mcp.clone(),
        &McpToolName("job.create".to_string()),
        create_input(),
    )
    .await
    .expect("create must succeed");
    let job_id: JobId = serde_json::from_value(create_out.get("job_id").cloned().unwrap()).unwrap();

    let reconcile_out = dispatch(
        mcp.clone(),
        &McpToolName("job.reconcile".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("reconcile must succeed");
    let current = reconcile_out
        .pointer("/outcome/no_op/current")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    assert_eq!(
        current.as_deref(),
        Some("event_detected"),
        "ReconcileOutcome::NoOp {{ current: \"event_detected\" }} expected, got: {reconcile_out}"
    );
}

#[tokio::test]
async fn dispatcher_resumes_a_job_awaiting_review() {
    // An agent that lands in `HumanReviewRequired` has exactly one
    // way forward, and it has to be reachable over MCP — the C5
    // driver holds no `RuntimeService`, so a resume command that
    // only existed in the runtime would strand the scenario.
    //
    // There is deliberately no matching `job.enter_human_review`
    // tool: escalation is an orchestration decision, and an agent
    // that could raise it against itself could also strand itself.
    // The test therefore enters via the runtime handle the harness
    // already has, and resumes via `dispatch`.
    let service = build_runtime();
    let mcp = McpRuntime::new(Arc::new(service));

    let create_out = dispatch(
        mcp.clone(),
        &McpToolName("job.create".to_string()),
        create_input(),
    )
    .await
    .expect("create must succeed");
    let job_id: JobId = serde_json::from_value(create_out.get("job_id").cloned().unwrap()).unwrap();

    // Resuming a job that is not awaiting review is refused, not
    // silently accepted.
    let err = dispatch(
        mcp.clone(),
        &McpToolName("job.resume".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect_err("a healthy job cannot be resumed");
    assert!(
        err.to_string().contains("not an exceptional state"),
        "got: {err}"
    );

    mcp.service()
        .handle_command(ironmaint_runtime::RuntimeCommand::EnterHumanReview {
            job_id,
            reason: "needs a human".to_string(),
        })
        .await
        .expect("enter human review");

    // The agent can see that resume is the move available.
    let next = dispatch(
        mcp.clone(),
        &McpToolName("job.next_actions".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("next_actions");
    assert_eq!(
        next.pointer("/actions/allowed/0"),
        Some(&serde_json::json!("resume_job")),
        "next_actions must offer the way out: {next}"
    );

    let resumed = dispatch(
        mcp.clone(),
        &McpToolName("job.resume".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("resume must succeed");
    assert_eq!(
        resumed.get("resumed_to").and_then(|v| v.as_str()),
        Some("event_detected"),
        "resumed to the recorded state, got: {resumed}"
    );

    // And the projection agrees, read through a second tool.
    let got = dispatch(
        mcp.clone(),
        &McpToolName("job.get".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("job.get");
    assert_eq!(
        got.pointer("/projection/state").and_then(|v| v.as_str()),
        Some("event_detected"),
        "got: {got}"
    );
}

#[tokio::test]
async fn dispatcher_rejects_unknown_tool() {
    let mcp = McpRuntime::new(Arc::new(build_runtime()));
    let err = dispatch(
        mcp.clone(),
        &McpToolName("does.not.exist".to_string()),
        serde_json::json!({}),
    )
    .await
    .expect_err("unknown tool must error");
    assert!(matches!(err, ironmaint_mcp::McpError::Other(_)));
}

#[tokio::test]
async fn dispatcher_round_trips_capture_through_runtime() {
    let tmp = tempfile::tempdir().expect("temp workspace root");
    let mcp = build_runtime_with_workspace(tmp.path());

    let create_out = dispatch(
        mcp.clone(),
        &McpToolName("job.create".to_string()),
        create_input(),
    )
    .await
    .expect("create must succeed");
    let job_id: JobId = serde_json::from_value(create_out.get("job_id").cloned().unwrap()).unwrap();

    let capture_out = dispatch(
        mcp.clone(),
        &McpToolName("candidate.capture".to_string()),
        serde_json::json!({
            "job_id": job_id,
            "package": debian_foo(),
            "repository_url": "https://example.invalid/foo.git",
        }),
    )
    .await
    .expect("capture must succeed");
    let fp = capture_out
        .get("fingerprint")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    assert!(
        fp.is_some_and(|s| !s.is_empty()),
        "non-empty fingerprint: {capture_out}"
    );
}

#[tokio::test]
async fn dispatcher_round_trips_workspace_stat() {
    let tmp = tempfile::tempdir().expect("temp workspace root");
    let mcp = build_runtime_with_workspace(tmp.path());
    // workspace.stat needs a job_id (the workspace is
    // attached to a job); we just create one and pass it.
    let create_out = dispatch(
        mcp.clone(),
        &McpToolName("job.create".to_string()),
        create_input(),
    )
    .await
    .expect("create must succeed");
    let job_id: JobId = serde_json::from_value(create_out.get("job_id").cloned().unwrap()).unwrap();

    let out = dispatch(
        mcp.clone(),
        &McpToolName("workspace.stat".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("workspace.stat must succeed");
    // A genuinely-provisioned empty workspace has never been
    // mutated, so its stored revision really is 0. Before 0B.9
    // this assertion passed for the wrong reason: the dispatcher
    // fabricated a `WorkspaceRevision::new()` without reading any
    // state. It is now backed by the store.
    let rev = out.get("revision").and_then(|v| v.as_u64());
    assert_eq!(rev, Some(0));
}

#[tokio::test]
async fn workspace_tools_report_a_typed_error_when_no_manager_is_configured() {
    // A deployment that forgets `with_workspace` must get an
    // actionable error, not a fabricated success.
    let mcp = McpRuntime::new(Arc::new(build_runtime()));
    let create_out = dispatch(
        mcp.clone(),
        &McpToolName("job.create".to_string()),
        create_input(),
    )
    .await
    .expect("create must succeed");
    let job_id: JobId = serde_json::from_value(create_out.get("job_id").cloned().unwrap()).unwrap();

    let err = dispatch(
        mcp,
        &McpToolName("workspace.stat".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect_err("stat without a workspace manager must error");
    let ironmaint_mcp::McpError::Internal(message) = err else {
        panic!("expected McpError::Internal, got {err:?}");
    };
    assert!(
        message.contains("with_workspace"),
        "the error must name the missing builder call: {message}"
    );
}

// Suppress unused-import warnings when the dispatcher test
// is built without the integration feature.
#[allow(unused_imports)]
use NullExecutor as _NullExecutorReexport;
