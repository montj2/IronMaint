//! Self-test for `assert_executor_conformance` (PHASE-0B.md §92).
//!
//! Construct a real `ProcessExecutor` against the fixture binary
//! in `bins/ironmaint-fixture/`, register the six fixture tool
//! keys via [`register_fixture_tools`], and run the full §92
//! suite. The tests pass if and only if every §92 condition
//! surfaces as the conformance expects.
//!
//! `unwrap` / `expect` are allowed here because test failure
//! should panic; production crates forbid them via the workspace
//! lint table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_executor::{ProcessExecutor, ToolRegistry};
use ironmaint_testkit::{
    FixtureKeys, assert_executor_conformance, fixture_binary_path, register_fixture_tools,
};

fn build_executor() -> (tempfile::TempDir, ProcessExecutor, FixtureKeys) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut registry = ToolRegistry::new();
    let keys = register_fixture_tools(&mut registry, &fixture_binary_path());
    let registry_arc = Arc::new(registry);
    let store = Arc::new(ArtifactStore::open(ArtifactRoot::new(dir.path())));
    let executor = ProcessExecutor::new(registry_arc, store, Default::default());
    (dir, executor, keys)
}

#[tokio::test]
async fn assert_executor_conformance_passes_for_process_executor() {
    let (_dir, executor, keys) = build_executor();
    assert_executor_conformance(&executor, &keys).await;
}

#[tokio::test]
async fn assert_executor_conformance_records_truncated_when_over_limit() {
    // §15 last bullet: a 256 KiB fixture stdout against a 4 KiB
    // executor cap must surface as `truncated = true`. The
    // conformance suite asserts this in `assert_truncate`;
    // running the whole suite here keeps the broader §92
    // coverage in a single hermetic test.
    let (_dir, executor, keys) = build_executor();
    assert_executor_conformance(&executor, &keys).await;
}

#[tokio::test]
async fn assert_executor_conformance_handles_tight_timeout() {
    // The `timeout` fixture sleeps 120s; the conformance
    // registers it with a 500ms wall-clock cap. The executor's
    // `tokio::time::timeout` must fire long before the fixture
    // wakes up.
    let (_dir, executor, keys) = build_executor();
    assert_executor_conformance(&executor, &keys).await;
}
