//! PR 1.1.I — `ProcessExecutor` env-boundary tests.
//!
//! The executor's `process.rs:255-264` builds a `Command`
//! with `tool.executable()` and adds env via `cmd.env(k, v)`,
//! but does NOT call `env_clear()`. The subprocess therefore
//! inherits the entire ambient env of the parent (the
//! ironmaintd daemon, or in tests, the test process). For
//! untrusted build scripts this is a leak: `IRONMAINT_*`,
//! `BTS_HELPER`, `AWS_*`, `LD_*`, proxy vars, etc. all flow
//! through to the tool.
//!
//! The fix (PR 1.1.I GREEN) is `cmd.env_clear()` + a per-tool
//! env allow-list, so the child sees only
//! `PATH`/`HOME`/`LC_ALL` (the `ProcessEnvironment` baseline)
//! + the tool's per-tool `env()` + the request's
//! `env_overrides`.
//!
//! The RED test in this file pins the *exact* set of env vars
//! the child must see after the fix. Before the fix the child
//! sees every `CARGO_*`, `RUSTC*`, `IRONMAINT_*` the test
//! process was started with, and the assertion fails.
//!
//! `printenv` is a stable POSIX utility (no flags needed: it
//! prints all env vars, one per line, `KEY=VALUE` format, and
//! exits 0). It is at `/usr/bin/printenv` on macOS and Linux
//! and on every CI runner this workspace targets.
//!
//! Reference: `doc/PHASE-1-COMPLETION.md` §1.1 row 1.1.I;
//! architect's post-release review.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRequest, Executor, ProcessExecutor, ToolDefinitionRecord,
    ToolRegistry,
};

fn job_id() -> ironmaint_core::JobId {
    ironmaint_core::JobId::new()
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

/// PR 1.1.I teeth-check: the executor's child process sees
/// ONLY the `ProcessEnvironment` baseline (`PATH`, `HOME`,
/// `LC_ALL`) plus the request's `env_overrides` — NOT the
/// ambient env of the parent (the test process, which has
/// `CARGO_*`, `RUSTC*`, etc. set by the test harness).
///
/// Before the fix (the RED commit) the executor builds the
/// `Command` with `cmd.env(k, v)` and no `env_clear()`, so
/// the child sees every ambient env var the test process
/// was started with. The assertion below pins the set to
/// the four closed keys, which fails today.
#[tokio::test]
async fn executor_subprocess_env_is_closed_allowlist() {
    let (_dir, store) = temp_store();
    let mut registry = ToolRegistry::new();
    let ck = ToolCapabilityKey::new("test.env.printenv").unwrap();
    let record = ToolDefinitionRecord::new(
        ck.clone(),
        std::path::PathBuf::from("/usr/bin/printenv"),
        vec![],
        ExecutionClass::Check,
        ExecutionLimits {
            timeout: Duration::from_secs(5),
            stdout_max_bytes: 64 * 1024,
            stderr_max_bytes: 64 * 1024,
        },
    );
    registry.register(Box::new(record)).expect("register");

    let exec = make_executor(Arc::new(registry), store);

    let mut req = ExecutionRequest::new(
        job_id(),
        ck,
        ironmaint_executor::RetryClass::Safe,
        serde_json::json!({}),
    );
    req.env_overrides
        .insert("BTS_HELPER".to_string(), "/fake/bts".to_string());

    let rec = exec.execute(req).await.expect("execute");
    assert_eq!(rec.exit_code, 0, "printenv exits 0");

    let keys: BTreeSet<String> = rec
        .stdout
        .lines()
        .filter_map(|line| line.split_once('=').map(|(k, _)| k.to_string()))
        .collect();

    let expected: BTreeSet<String> = [
        "PATH".to_string(),
        "HOME".to_string(),
        "LC_ALL".to_string(),
        "BTS_HELPER".to_string(),
    ]
    .into_iter()
    .collect();

    assert_eq!(
        keys, expected,
        "executor child must see only baseline + request overrides; \
         got {} keys (e.g. {:?}), expected {} keys ({:?})",
        keys.len(),
        keys.iter().take(5).collect::<Vec<_>>(),
        expected.len(),
        expected,
    );
}
