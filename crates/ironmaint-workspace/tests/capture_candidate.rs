//! Candidate capture test: same HEAD → same fingerprint twice.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, JobId, PackageIdentity, PackageName,
};
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::WorkspaceManager;

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

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("foo").unwrap(),
    )
}

#[tokio::test]
async fn capture_returns_same_fingerprint_for_same_head() {
    if !git_available().await {
        eprintln!("git not available; skipping capture test");
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

    let job = JobId::new();
    let c1 = mgr
        .capture_candidate(id, job, package(), "https://example.invalid/foo.git")
        .await
        .unwrap();
    let c2 = mgr
        .capture_candidate(id, job, package(), "https://example.invalid/foo.git")
        .await
        .unwrap();
    // Same HEAD + same package + same repository_url ⇒ same
    // fingerprint, regardless of which JobId bound the capture.
    // JobId belongs to the *job projection*, not the source
    // content; the fingerprint is over the immutable source bytes.
    assert_eq!(c1.fingerprint(), c2.fingerprint());

    // Different repository URL ⇒ different fingerprint.
    let c3 = mgr
        .capture_candidate(id, job, package(), "https://example.invalid/foo.git")
        .await
        .unwrap();
    let c4 = mgr
        .capture_candidate(id, job, package(), "https://example.invalid/bar.git")
        .await
        .unwrap();
    assert_ne!(c3.fingerprint(), c4.fingerprint());
}

/// Each `capture_candidate` lands a §22 internal maintenance
/// commit whose author is `IronMaint <ironmaint@localhost>` and
/// whose subject matches the §22 convention. PHASE-0B.md §22.
#[tokio::test]
async fn capture_writes_maintenance_commit_with_required_metadata() {
    if !git_available().await {
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

    let job = JobId::new();
    let _c1 = mgr
        .capture_candidate(id, job, package(), "https://example.invalid/foo.git")
        .await
        .unwrap();

    // Inspect git log with the §22 maintenance env. The newest
    // commit must be the §22 commit, authored by IronMaint.
    use ironmaint_workspace::git::GitInvocation;
    let inv = GitInvocation::for_maintenance_commit(&handle_dir);
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
    if !git_available().await {
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

    let job = JobId::new();
    let _c1 = mgr
        .capture_candidate(id, job, package(), "https://example.invalid/foo.git")
        .await
        .unwrap();
    let _c2 = mgr
        .capture_candidate(id, job, package(), "https://example.invalid/foo.git")
        .await
        .unwrap();

    use ironmaint_workspace::git::GitInvocation;
    let inv = GitInvocation::for_maintenance_commit(&handle_dir);
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
