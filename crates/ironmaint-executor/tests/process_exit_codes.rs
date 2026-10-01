//! §92 exit-checkpoint tests for `ProcessExecutor`. Drives the
//! real `ironmaint-fixture` binary through `ProcessExecutor` and
//! asserts the documented exit-code taxonomy.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_executor::fixture::Outcome;
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRequest, Executor, ExecutorErrorKind,
    ProcessExecutor, RetryClass, ToolDefinitionRecord, ToolRegistry, outcome_from_record,
};
use serde_json::json;

/// A job id for an execution.
///
/// The executor's artifact guard is per-job (§15), so a request has
/// to name one; the id itself is not what any of these tests are
/// about.
fn job_id() -> ironmaint_core::JobId {
    ironmaint_core::JobId::new()
}

fn fixture_bin() -> PathBuf {
    // `CARGO_BIN_EXE_<name>` is only set for tests inside the
    // crate that owns the binary. Cross-crate tests resolve
    // the path relative to the workspace target dir. This
    // works for `cargo test` run from the workspace root.
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");
    let mut path = workspace_dir
        .join("target")
        .join("debug")
        .join("ironmaint-fixture");
    if !path.exists() {
        // Fall back to release profile.
        path = workspace_dir
            .join("target")
            .join("release")
            .join("ironmaint-fixture");
    }
    path
}

fn temp_store() -> (tempfile::TempDir, Arc<ArtifactStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ArtifactStore::open(ArtifactRoot::new(dir.path())));
    (dir, store)
}

/// Register the fixture binary as a tool with the given tool_key
/// and per-tool limits. The tool_key is passed as argv[1] (the
/// fixture's argv interface).
fn register_fixture(
    registry: &mut ToolRegistry,
    key: &str,
    limits: ExecutionLimits,
) -> ToolCapabilityKey {
    let ck = ToolCapabilityKey::new(key).unwrap();
    let rec = ToolDefinitionRecord::new(
        ck.clone(),
        fixture_bin(),
        vec![OsString::from(key)],
        ExecutionClass::Check,
        limits,
    );
    registry.register(Box::new(rec)).expect("register");
    ck
}

fn make_executor(registry: Arc<ToolRegistry>, store: Arc<ArtifactStore>) -> ProcessExecutor {
    ProcessExecutor::new(registry, store, Default::default()).with_time_factory(Arc::new(|| {
        time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }))
}

#[tokio::test]
async fn pass_path_returns_zero_exit() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = register_fixture(
        &mut r,
        "synthetic.build.validate",
        ExecutionLimits::default(),
    );
    let exec = make_executor(Arc::new(r), store);

    let rec = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("PASS");
    assert_eq!(rec.exit_code, 0);
    assert!(!rec.truncated);
    assert!(rec.stdout.contains("BUILD OK"));
    assert_eq!(outcome_from_record(&rec), Outcome::Pass);
}

#[tokio::test]
async fn fail_path_returns_one_exit() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = register_fixture(&mut r, "synthetic.build.fail", ExecutionLimits::default());
    let exec = make_executor(Arc::new(r), store);

    let rec = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("FAIL is a record, not an error");
    assert_eq!(rec.exit_code, 1);
    assert_eq!(outcome_from_record(&rec), Outcome::Fail);
}

#[tokio::test]
async fn timeout_kills_child() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = register_fixture(
        &mut r,
        "synthetic.build.timeout",
        ExecutionLimits {
            timeout: Duration::from_millis(500),
            stdout_max_bytes: 1024,
            stderr_max_bytes: 1024,
        },
    );
    let exec = make_executor(Arc::new(r), store);

    let err = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect_err("must time out");
    assert_eq!(
        err.kind,
        ExecutorErrorKind::ToolFailed { timed_out: true },
        "expected timed_out=true, got {err:?}"
    );
}

#[tokio::test]
async fn interrupt_path_returns_130() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = register_fixture(
        &mut r,
        "synthetic.build.interrupt",
        ExecutionLimits::default(),
    );
    let exec = make_executor(Arc::new(r), store);

    let rec = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("INTERRUPT is a record, not an error");
    assert_eq!(rec.exit_code, 130);
    assert_eq!(outcome_from_record(&rec), Outcome::Interrupted);
}

#[tokio::test]
async fn infra_fail_exit_maps_to_infrastructure_outcome() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = register_fixture(
        &mut r,
        "synthetic.build.infra_fail",
        ExecutionLimits::default(),
    );
    let exec = make_executor(Arc::new(r), store);

    let rec = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("exit 127 is a record");
    assert_eq!(rec.exit_code, 127);
    assert_eq!(outcome_from_record(&rec), Outcome::InfrastructureFailed);
}

#[tokio::test]
async fn missing_executable_is_infra_failure() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = ToolCapabilityKey::new("synthetic.build.ghost").unwrap();
    let rec = ToolDefinitionRecord::new(
        key.clone(),
        PathBuf::from("/this/executable/does/not/exist/at/all"),
        vec![OsString::from("synthetic.build.ghost")],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    );
    r.register(Box::new(rec)).expect("register");
    let exec = make_executor(Arc::new(r), store);

    let err = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect_err("spawn must fail");
    assert_eq!(
        err.kind,
        ExecutorErrorKind::InfrastructureFailed,
        "expected InfrastructureFailed, got {err:?}"
    );
}

#[tokio::test]
async fn unknown_capability_is_rejected() {
    let (_dir, store) = temp_store();
    let r = Arc::new(ToolRegistry::new());
    let exec = make_executor(r, store);

    let key = ToolCapabilityKey::new("synthetic.build.absent").unwrap();
    let err = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, ExecutorErrorKind::UnknownCapability);
}

#[tokio::test]
async fn privileged_external_class_is_rejected() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = ToolCapabilityKey::new("synthetic.build.priv").unwrap();
    let rec = ToolDefinitionRecord::new(
        key.clone(),
        fixture_bin(),
        vec![OsString::from("synthetic.build.validate")],
        ExecutionClass::PrivilegedExternal,
        ExecutionLimits::default(),
    );
    r.register(Box::new(rec)).expect("register");
    let exec = make_executor(Arc::new(r), store);

    let err = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect_err("must reject PrivilegedExternal");
    assert!(
        matches!(err.kind, ExecutorErrorKind::Other(ref s) if s.contains("PrivilegedExternal")),
        "expected Other(\"PrivilegedExternal...\"), got {err:?}"
    );
}

#[tokio::test]
async fn truncated_records_propagate_to_execution_record() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = register_fixture(
        &mut r,
        "synthetic.build.truncate",
        ExecutionLimits {
            timeout: Duration::from_secs(60),
            stdout_max_bytes: 4096,
            stderr_max_bytes: 1024,
        },
    );
    let exec = make_executor(Arc::new(r), store);

    let rec = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("truncate run produces a record");
    assert_eq!(rec.exit_code, 0);
    assert!(rec.truncated, "stdout was 256 KiB vs 4 KiB cap");
    assert_eq!(rec.stdout.len(), 4096);
    assert_eq!(outcome_from_record(&rec), Outcome::Pass);
}

#[tokio::test]
async fn passthrough_when_under_limit_is_not_truncated() {
    let (_dir, store) = temp_store();
    let mut r = ToolRegistry::new();
    let key = register_fixture(
        &mut r,
        "synthetic.build.truncate",
        ExecutionLimits {
            timeout: Duration::from_secs(60),
            stdout_max_bytes: 1024 * 1024, // 1 MiB cap, fixture emits 256 KiB
            stderr_max_bytes: 1024 * 1024,
        },
    );
    let exec = make_executor(Arc::new(r), store);

    let rec = exec
        .execute(ExecutionRequest::new(
            job_id(),
            key,
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("truncate run");
    assert!(!rec.truncated, "256 KiB < 1 MiB cap → not truncated");
    assert_eq!(rec.stdout.len(), 256 * 1024);
}
