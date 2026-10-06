//! PR 1A.2 — `ToolInputMode::JsonStdin` end-to-end tests.
//!
//! The executor advertises an input mode on every `ToolDefinition`.
//! `None` is the default — the child gets `Stdio::null()`, the
//! current behaviour, and nothing changes for the six synthetic
//! tools. `JsonStdin` is the new one: the executor serialises the
//! `ExecutionRequest::input` as JSON, writes it to the child's
//! stdin, and closes the pipe. The fixture binary keys
//! `synthetic.build.echo_input` reads stdin to stdout and exits 0,
//! so a test that runs that tool with a non-trivial `input` can
//! assert the bytes arrived in the right shape.
//!
//! The teeth-check is `json_stdin_sends_payload_to_child`: it
//! proves the wiring works end-to-end through the real executor.
//! Before PR 1A.2 the fixture has no `echo_input` key and the
//! `input_mode()` accessor does not exist, so the test does not
//! compile — the failure is a compile error rather than a runtime
//! one, which is the right shape for a wiring test.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRequest, Executor, ProcessExecutor, ToolDefinition,
    ToolDefinitionRecord, ToolInputMode, ToolRegistry,
};
use serde_json::json;

fn job_id() -> ironmaint_core::JobId {
    ironmaint_core::JobId::new()
}

fn fixture_bin() -> PathBuf {
    ironmaint_fixture::fixture_binary_path()
}

fn temp_store() -> (tempfile::TempDir, Arc<ArtifactStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ArtifactStore::open(ArtifactRoot::new(dir.path())));
    (dir, store)
}

fn make_executor(registry: Arc<ToolRegistry>, store: Arc<ArtifactStore>) -> ProcessExecutor {
    ProcessExecutor::new(registry, store, Default::default()).with_time_factory(Arc::new(|| {
        time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }))
}

/// PR 1A.2 teeth-check: a tool registered with `JsonStdin` receives
/// the runtime-built input payload on stdin, and the child can
/// echo it back through stdout.
///
/// Before this PR the fixture binary has no `echo_input` key and
/// `ToolDefinitionRecord` has no `with_input_mode` builder, so
/// neither this assertion nor the build itself compiles.
#[tokio::test]
async fn json_stdin_sends_payload_to_child() {
    let (_dir, store) = temp_store();
    let mut registry = ToolRegistry::new();
    let ck = ToolCapabilityKey::new("synthetic.build.echo_input").unwrap();
    let record = ToolDefinitionRecord::new(
        ck.clone(),
        fixture_bin(),
        vec![OsString::from("synthetic.build.echo_input")],
        ExecutionClass::Check,
        ExecutionLimits {
            timeout: Duration::from_secs(5),
            stdout_max_bytes: 64 * 1024,
            stderr_max_bytes: 64 * 1024,
        },
    )
    .with_input_mode(ToolInputMode::JsonStdin);
    registry.register(Box::new(record)).expect("register");

    let exec = make_executor(Arc::new(registry), store);

    let payload = json!({
        "job_id": "00000000-0000-0000-0000-000000000001",
        "package": {"family": "debian", "name": "hello"},
        "commit": "deadbeef",
        "fingerprint": "blake3:0000",
    });

    let rec = exec
        .execute(ExecutionRequest::new(
            job_id(),
            ck,
            ironmaint_executor::RetryClass::Safe,
            payload.clone(),
        ))
        .await
        .expect("execute");

    assert_eq!(rec.exit_code, 0, "echo_input exits 0");
    let parsed: serde_json::Value =
        serde_json::from_str(&rec.stdout).expect("echo_input writes valid JSON");
    assert_eq!(
        parsed, payload,
        "the child must receive the exact input payload"
    );
}

/// `ToolInputMode::default()` is `None` so existing registration
/// sites (and any third-party `ToolDefinition` implementer) keep
/// today's `Stdio::null()` behaviour — the new wiring is opt-in,
/// not a behaviour change for the six synthetic tools.
#[test]
fn default_input_mode_is_none() {
    let r = ToolDefinitionRecord::new(
        ToolCapabilityKey::new("synthetic.build.validate").unwrap(),
        PathBuf::from("/bin/true"),
        vec![],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    );
    assert_eq!(r.input_mode(), ToolInputMode::None);
}
