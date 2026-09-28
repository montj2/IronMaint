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
    let mcp = McpRuntime::new(Arc::new(build_runtime()));

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
    let mcp = McpRuntime::new(Arc::new(build_runtime()));
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
    // `WorkspaceRevision::ZERO` is `0`, so the dispatcher must
    // return revision=0 for a fresh workspace.
    let rev = out.get("revision").and_then(|v| v.as_u64());
    assert_eq!(rev, Some(0));
}

// Suppress unused-import warnings when the dispatcher test
// is built without the integration feature.
#[allow(unused_imports)]
use NullExecutor as _NullExecutorReexport;
