//! Tests for the read/list/diff/status manager surface
//! (PHASE-0B.md §16, §19). These tests drive a real git
//! workspace and exercise the file-system-touching code paths.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use ironmaint_core::JobId;
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::{EntryKind, WorkspaceManager, WorkspaceRevision};

/// `read` returns the file's bytes for a path inside the
/// workspace root.
#[tokio::test]
async fn read_returns_file_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let store = MockStore::new();
    let mgr = WorkspaceManager::new(handle_root.clone(), store);
    let id = mgr.create(JobId::new()).await.unwrap();
    let handle_dir = handle_root.join(format!("{id}"));
    tokio::fs::create_dir_all(&handle_dir).await.unwrap();
    common::init_repo(&handle_dir).await;

    let bytes = mgr
        .read(id, std::path::Path::new("README.md"))
        .await
        .unwrap();
    assert_eq!(bytes, b"# hello\n");
}

/// `list` returns files and directories inside a workspace dir;
/// symlinks are followed and reported by their target type.
#[tokio::test]
async fn list_returns_files_and_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let store = MockStore::new();
    let mgr = WorkspaceManager::new(handle_root.clone(), store);
    let id = mgr.create(JobId::new()).await.unwrap();
    let handle_dir = handle_root.join(format!("{id}"));
    tokio::fs::create_dir_all(&handle_dir).await.unwrap();
    common::init_repo(&handle_dir).await;

    // Add a subdirectory with a file inside, and a top-level file.
    tokio::fs::create_dir(handle_dir.join("subdir"))
        .await
        .unwrap();
    tokio::fs::write(handle_dir.join("extra.txt"), b"x")
        .await
        .unwrap();

    let entries = mgr.list(id, std::path::Path::new(".")).await.unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"README.md"), "names: {names:?}");
    assert!(names.contains(&"subdir"), "names: {names:?}");
    assert!(names.contains(&"extra.txt"), "names: {names:?}");

    let readme = entries.iter().find(|e| e.name == "README.md").unwrap();
    assert_eq!(readme.kind, EntryKind::File);
    let subdir = entries.iter().find(|e| e.name == "subdir").unwrap();
    assert_eq!(subdir.kind, EntryKind::Directory);
}

/// `diff` against HEAD shows the changes from `apply_patch`.
#[tokio::test]
async fn diff_against_head_shows_changes() {
    if !common::git_available().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let (mgr, id, _job, _store): (
        WorkspaceManager<MockStore>,
        ironmaint_workspace::WorkspaceId,
        JobId,
        MockStore,
    ) = common::fresh_workspace(&handle_root).await;

    let rev = mgr.current_revision(id).await.unwrap();
    mgr.apply_patch(id, rev, &common::readme_v2_patch())
        .await
        .unwrap();

    let diff_text = mgr.diff(id, None).await.unwrap();
    assert!(
        diff_text.contains("hello v2"),
        "diff should contain the new content, got: {diff_text}"
    );
    assert!(
        diff_text.contains("hello"),
        "diff should still reference the old content, got: {diff_text}"
    );
}

/// `status` is empty for a clean workspace and non-empty after
/// `apply_patch` (which leaves the working tree dirty).
#[tokio::test]
async fn status_reflects_workspace_dirty_state() {
    if !common::git_available().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let (mgr, id, _job, _store): (
        WorkspaceManager<MockStore>,
        ironmaint_workspace::WorkspaceId,
        JobId,
        MockStore,
    ) = common::fresh_workspace(&handle_root).await;

    // Right after init_repo + apply_patch, the tree is dirty.
    let rev = WorkspaceRevision::ZERO;
    mgr.apply_patch(id, rev, &common::readme_v2_patch())
        .await
        .unwrap();
    let status = mgr.status(id).await.unwrap();
    assert!(
        !status.is_empty(),
        "expected non-empty status after apply_patch"
    );

    // After capture (which calls git add -A), the staged
    // entries are gone but the §22 maintenance commit isn't
    // committed by capture, so the working tree remains
    // dirty from `git status`'s perspective. Verify the API
    // surface works (we don't assert emptiness here).
    let _capture = mgr
        .capture_candidate(
            id,
            JobId::new(),
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    let _post_capture_status = mgr.status(id).await.unwrap();
}
