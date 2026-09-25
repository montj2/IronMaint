//! Smoke tests for the executor skeleton (commit 9).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_executor::{
    ExecutionRequest, Executor, NullExecutor, ProcessEnvironment, RetryClass,
};
use serde_json::json;

#[tokio::test]
async fn null_executor_rejects() {
    let ex = NullExecutor;
    let req = ExecutionRequest::new(
        ToolCapabilityKey::new("synthetic.build.validate").unwrap(),
        RetryClass::Idempotent,
        json!({}),
    );
    let err = ex.execute(req).await.expect_err("must reject");
    assert_eq!(
        err.kind,
        ironmaint_executor::ExecutorErrorKind::UnknownCapability
    );
}

#[tokio::test]
async fn process_environment_baseline() {
    let env = ProcessEnvironment::new();
    assert!(env.get("PATH").is_some());
    assert!(env.get("HOME").is_some());
    assert!(env.get("LC_ALL").is_some());
    assert_eq!(env.len(), 3);
}

#[test]
fn destructive_has_zero_retries() {
    assert_eq!(RetryClass::Destructive.max_retries(), Some(0));
}
