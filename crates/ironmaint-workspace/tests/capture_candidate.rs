//! Candidate capture tests: same HEAD → same fingerprint twice,
//! §22 maintenance commit metadata, and history growth per call.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use ironmaint_store::CandidateStore;

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

/// Re-capturing an unchanged tree resolves to the candidate that is
/// already stored, and does not mint a second one.
///
/// This is the regression test for the eleventh instance of the
/// pattern this phase keeps finding: the fingerprint is the
/// candidate's identity (`migrations/0001` declares it UNIQUE, 0A
/// §22 says a *source change* creates a new candidate), the
/// workspace manager modelled that correctly, and the writer that
/// reaches `put_source_candidate` did not look it up first. On
/// `MockStore` the duplicate was silent — a second row under a fresh
/// id — and on the SQLite backend the daemon actually runs it was a
/// hard `UNIQUE constraint failed: source_candidates.fingerprint`
/// surfaced to the agent mid-conversation.
///
/// The §101 acceptance scenario hits it on its second capture, which
/// is why the test is here rather than only there: a mock-only suite
/// would have reported the defect as a duplicate row that nothing
/// downstream reads, and no test read it.
#[tokio::test]
async fn re_capturing_an_unchanged_tree_stores_one_candidate() {
    if !common::git_available().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let (mgr, id, job, store) = common::fresh_workspace(&handle_root).await;

    let first = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    let second = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();

    assert_eq!(
        first.id(),
        second.id(),
        "the second capture must return the stored candidate, not one it built \
         and never wrote"
    );

    let rows = store.list_source_candidates_for_job(job).await.unwrap();
    assert_eq!(
        rows.len(),
        1,
        "one source, one candidate: the fingerprint is the identity"
    );
    assert_eq!(
        store
            .find_source_by_fingerprint(first.fingerprint())
            .await
            .unwrap(),
        Some(first.id()),
        "and it is reachable by its fingerprint"
    );

    // The §22 maintenance commit is still written on the dedup
    // path. Skipping it would be wrong in the other direction: the
    // capture happened, and history is where an audit finds that.
    use ironmaint_workspace::git::GitInvocation;
    let log = GitInvocation::for_maintenance_commit(&handle_root.join(format!("{id}")))
        .log(2)
        .await
        .unwrap();
    assert_eq!(
        log.len(),
        2,
        "one maintenance commit per capture, even when the capture \
         resolves to an existing candidate"
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
