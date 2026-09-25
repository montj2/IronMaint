//! Tests for `WorkspaceManager::apply_patch`.
//!
//! Skips if `git` is not available (so CI without a git
//! installation still passes the workspace conformance surface).
//! Requires a real git repository initialised under
//! `<tmp>/<handle>` so `git apply` has something to diff
//! against.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::JobId;
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::WorkspaceManager;
use ironmaint_workspace::WorkspaceRevision;

async fn git_available() -> bool {
    let out = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await;
    matches!(out, Ok(o) if o.status.success())
}

async fn init_repo(path: &std::path::Path) {
    let run = |args: &[&str]| {
        let p = path.to_path_buf();
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        async move {
            tokio::process::Command::new("git")
                .args(&v)
                .current_dir(&p)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", "/var/empty")
                .output()
                .await
        }
    };
    let _ = run(&["init"]).await;
    let _ = run(&["config", "user.email", "test@invalid"]).await;
    let _ = run(&["config", "user.name", "ironmaint-test"]).await;
    tokio::fs::write(path.join("README.md"), "# hello\n")
        .await
        .unwrap();
    let _ = run(&["add", "."]).await;
    let _ = run(&["commit", "-m", "initial"]).await;
}

#[tokio::test]
async fn apply_patch_bumps_revision_when_clean() {
    if !git_available().await {
        eprintln!("git not available; skipping apply_patch test");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let store = MockStore::new();
    let mgr = WorkspaceManager::new(handle_root.clone(), store);

    let id = mgr.create(JobId::new()).await.unwrap();
    let handle_dir = handle_root.join(format!("{id}"));
    tokio::fs::create_dir_all(&handle_dir).await.unwrap();
    init_repo(&handle_dir).await;

    // Add the patched file to the repo first so `git apply` has a
    // clean target to operate on.
    tokio::fs::write(handle_dir.join("note.txt"), "old\n")
        .await
        .unwrap();
    let _ = tokio::process::Command::new("git")
        .args(["add", "note.txt"])
        .current_dir(&handle_dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .output()
        .await
        .unwrap();
    let _ = tokio::process::Command::new("git")
        .args(["commit", "-m", "note"])
        .current_dir(&handle_dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .output()
        .await
        .unwrap();

    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n";
    let next = mgr
        .apply_patch(id, WorkspaceRevision::ZERO, patch)
        .await
        .unwrap();
    assert_eq!(next, WorkspaceRevision(1));
    let on_disk = tokio::fs::read_to_string(handle_dir.join("note.txt"))
        .await
        .unwrap();
    assert_eq!(on_disk, "new\n");
}

#[tokio::test]
async fn apply_patch_rejects_stale_revision() {
    if !git_available().await {
        eprintln!("git not available; skipping apply_patch cas test");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let store = MockStore::new();
    let mgr = WorkspaceManager::new(handle_root.clone(), store);
    let id = mgr.create(JobId::new()).await.unwrap();
    let handle_dir = handle_root.join(format!("{id}"));
    tokio::fs::create_dir_all(&handle_dir).await.unwrap();
    init_repo(&handle_dir).await;

    let err = mgr
        .apply_patch(id, WorkspaceRevision(99), "patch content")
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("conflict") || msg.contains("Conflict"),
        "expected conflict, got: {msg}"
    );
}
