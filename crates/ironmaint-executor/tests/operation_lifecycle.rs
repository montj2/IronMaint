//! Recovery semantics tests for OperationLifecycle.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ironmaint_core::{CandidateFingerprint, JobId, OperationId};
use ironmaint_executor::OperationLifecycle;
use ironmaint_policy::{AuthorizationState, PrivilegedOperation, PrivilegedOperationKind};
use ironmaint_store::OperationStore;
use ironmaint_store::mock::MockStore;

async fn setup_with_executing() -> (Arc<MockStore>, OperationId) {
    let store = Arc::new(MockStore::new());
    let op = PrivilegedOperation::proposed(
        PrivilegedOperationKind::CanonicalRepositoryPush,
        CandidateFingerprint::from_hex(
            "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20".to_string(),
        )
        .unwrap(),
    );
    let id = store.put_operation(&op, JobId::new()).await.expect("put");
    // Promote to Executing manually to simulate a stuck op.
    let mut promoted = op;
    promoted.authorization = AuthorizationState::Executing;
    store.update_operation(id, &promoted).await.expect("update");
    (store, id)
}

#[tokio::test]
async fn recover_inspects_zero_when_no_operations() {
    let store = Arc::new(MockStore::new());
    let lifecycle = OperationLifecycle::new(store);
    let report = lifecycle.recover_interrupted().await.unwrap();
    assert_eq!(report.inspected, 0);
    assert_eq!(report.recovered, 0);
    assert_eq!(report.abandoned, 0);
}

#[tokio::test]
async fn recover_reports_inspection_count() {
    let (store, _id) = setup_with_executing().await;
    let lifecycle = OperationLifecycle::new(store.clone());
    let report = lifecycle.recover_interrupted().await.unwrap();
    // The mock returns at most one ID per call; we may inspect
    // more than once but the test verifies the counter wiring.
    assert!(report.inspected >= 1);
}
