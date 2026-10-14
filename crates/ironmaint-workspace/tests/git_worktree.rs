//! Tests for the frozen-worktree primitive (PHASE-1.1 PR 1.1.B,
//! evidence integrity).
//!
//! The `check.run` path operates against a `git worktree add
//! --detach` snapshot of the captured tree, so a concurrent
//! `apply_patch` to the live workspace cannot change the
//! evidence's binding to that tree. The tests below pin down
//! the primitive: the worktree is created at the right path,
//! the worktree's tree OID matches the requested tree, the
//! worktree's working copy has the right files, and the
//! `Drop` impl tears the worktree down.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use ironmaint_core::JobId;
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::WorkspaceManager;

#[tokio::test]
async fn frozen_for_check_creates_a_readonly_checkout_at_the_captured_tree() {
    if !common::git_available().await {
        eprintln!("git not available; skipping worktree test");
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
    common::init_repo(&handle_dir).await;

    // The seed commit the workspace was created with
    // (`common::init_repo`'s "initial" commit) is the tree
    // we'll request. Read the head commit, then read its
    // tree — `git rev-parse <commit>^{tree}`.
    let head_commit = {
        let out = tokio::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&handle_dir)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", "/var/empty")
            .output()
            .await
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let head_tree = {
        let out = tokio::process::Command::new("git")
            .args(["rev-parse", &format!("{head_commit}^{{tree}}")])
            .current_dir(&handle_dir)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", "/var/empty")
            .output()
            .await
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let worktree_id = "test-check-1";
    let frozen = mgr
        .frozen_for_check(id, worktree_id, &head_tree)
        .await
        .expect("frozen_for_check materialises a worktree");

    // The worktree is at <handle>/.ironmaint-checks/<worktree_id>.
    let expected = handle_dir.join(".ironmaint-checks").join(worktree_id);
    assert_eq!(frozen.path(), expected, "worktree path matches the runtime contract");

    // The worktree's tree OID matches the captured tree.
    let actual_tree = frozen
        .tree_sha()
        .await
        .expect("worktree's HEAD^{tree} is readable");
    assert_eq!(
        actual_tree, head_tree,
        "frozen worktree's tree matches the candidate's tree"
    );

    // The worktree's working copy contains the captured
    // tree's files. The seed repo has a `README.md`.
    let readme = tokio::fs::read_to_string(frozen.path().join("README.md"))
        .await
        .expect("README.md is present in the frozen worktree");
    assert_eq!(readme, "# hello\n");

    // The worktree is a git worktree (not a fresh clone): the
    // main repo's `.git/worktrees/<worktree_id>` is present.
    let worktrees_dir = handle_dir.join(".git").join("worktrees");
    assert!(
        tokio::fs::metadata(&worktrees_dir).await.is_ok(),
        "the main repo's `.git/worktrees/` exists"
    );

    // Drop the frozen worktree; the worktree directory is
    // removed and the `.git/worktrees/<worktree_id>` is
    // cleaned up by `git worktree remove --force`.
    drop(frozen);
    assert!(
        !expected.exists(),
        "the worktree directory is removed on Drop"
    );
}

#[tokio::test]
async fn frozen_for_check_removes_a_stale_worktree_before_recreating() {
    if !common::git_available().await {
        eprintln!("git not available; skipping worktree test");
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
    common::init_repo(&handle_dir).await;

    let head_tree = {
        let out = tokio::process::Command::new("git")
            .args(["rev-parse", "HEAD^{tree}"])
            .current_dir(&handle_dir)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", "/var/empty")
            .output()
            .await
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // Pre-create a stale directory at the worktree path so
    // the second call has to remove it. The behaviour the
    // test pins is: a crashed run's leftover is not a failure
    // for the next check.
    let worktree_id = "test-check-2";
    let checks_dir = handle_dir.join(".ironmaint-checks");
    tokio::fs::create_dir_all(&checks_dir).await.unwrap();
    let stale = checks_dir.join(worktree_id);
    tokio::fs::create_dir_all(&stale).await.unwrap();
    tokio::fs::write(stale.join("stale.txt"), "stale")
        .await
        .unwrap();

    let _frozen = mgr
        .frozen_for_check(id, worktree_id, &head_tree)
        .await
        .expect("frozen_for_check recycles a stale worktree directory");
    drop(_frozen);
}
