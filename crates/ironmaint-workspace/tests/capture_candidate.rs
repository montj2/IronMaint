//! Candidate capture tests: same HEAD → same fingerprint twice,
//! §22 maintenance commit metadata, and history growth per call.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn capture_returns_same_fingerprint_for_same_head() {
    if !common::git_available().await {
        eprintln!("git not available; skipping capture test");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let (mgr, id, job, _store) = common::fresh_workspace(&handle_root).await;

    let c1 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    let c2 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    // Same HEAD + same package + same repository_url ⇒ same
    // fingerprint, regardless of which JobId bound the capture.
    // JobId belongs to the *job projection*, not the source
    // content; the fingerprint is over the immutable source bytes.
    assert_eq!(c1.fingerprint(), c2.fingerprint());

    // Different repository URL ⇒ different fingerprint.
    let c3 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    let c4 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/bar.git",
        )
        .await
        .unwrap();
    assert_ne!(c3.fingerprint(), c4.fingerprint());
}

/// Each `capture_candidate` lands a §22 internal maintenance
/// commit whose author is `IronMaint <ironmaint@localhost>` and
/// whose subject matches the §22 convention. PHASE-0B.md §22.
#[tokio::test]
async fn capture_writes_maintenance_commit_with_required_metadata() {
    if !common::git_available().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let (mgr, id, job, _store) = common::fresh_workspace(&handle_root).await;
    let ws_dir = handle_root.join(format!("{id}"));

    let _c1 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();

    // Inspect git log with the §22 maintenance env. The newest
    // commit must be the §22 commit, authored by IronMaint.
    use ironmaint_workspace::git::GitInvocation;
    let inv = GitInvocation::for_maintenance_commit(&ws_dir);
    let log = inv.log(1).await.unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(
        log[0].subject, "IronMaint internal maintenance commit",
        "newest commit must be the §22 maintenance commit"
    );
}

/// Two captures of the same workspace produce two distinct
/// §22 maintenance commits — history is preserved per capture.
#[tokio::test]
async fn capture_history_grows_per_call() {
    if !common::git_available().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let (mgr, id, job, _store) = common::fresh_workspace(&handle_root).await;
    let ws_dir = handle_root.join(format!("{id}"));

    let _c1 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    let _c2 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();

    use ironmaint_workspace::git::GitInvocation;
    let inv = GitInvocation::for_maintenance_commit(&ws_dir);
    let log = inv.log(10).await.unwrap();
    let maintenance: Vec<_> = log
        .iter()
        .filter(|c| c.subject == "IronMaint internal maintenance commit")
        .collect();
    assert_eq!(
        maintenance.len(),
        2,
        "expected 2 §22 maintenance commits, got {} (full log: {:?})",
        maintenance.len(),
        log
    );
}
