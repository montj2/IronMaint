//! Workspace manager revision-CAS tests against an in-memory mock store.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::JobId;
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::{WorkspaceManager, WorkspaceRevision};

#[tokio::test]
async fn revision_zero_default_and_bump() {
    let tmp = tempfile::tempdir().unwrap();
    let store = MockStore::new();
    let mgr = WorkspaceManager::new(tmp.path(), store);
    let jid = JobId::new();
    let id = mgr.create(jid).await.unwrap();
    let cur = mgr.current_revision(id).await.unwrap();
    assert_eq!(cur, WorkspaceRevision::ZERO);
    let next = mgr
        .bump_revision(id, WorkspaceRevision::ZERO)
        .await
        .unwrap();
    assert_eq!(next, WorkspaceRevision(1));
}

#[tokio::test]
async fn cas_failure_on_stale_expected() {
    let tmp = tempfile::tempdir().unwrap();
    let store = MockStore::new();
    let mgr = WorkspaceManager::new(tmp.path(), store);
    let id = mgr.create(JobId::new()).await.unwrap();
    let _first = mgr
        .bump_revision(id, WorkspaceRevision::ZERO)
        .await
        .unwrap();
    let err = mgr
        .bump_revision(id, WorkspaceRevision::ZERO)
        .await
        .unwrap_err();
    // The MockStore currently returns a Conflict for CAS failures;
    // we forward that into WorkspaceErrorKind::Conflict.
    let msg = format!("{err}");
    assert!(
        msg.contains("conflict") || msg.contains("Conflict"),
        "expected conflict, got {msg}"
    );
}
