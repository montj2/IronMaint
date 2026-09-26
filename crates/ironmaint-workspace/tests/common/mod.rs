//! Shared fixture for the workspace integration tests
//! (`capture_candidate`, `apply_patch`, `multi_capture_history`).
//!
//! Each test that needs a real git workspace runs `init_repo`
//! inside a tempdir handle directory, then drives the manager
//! through `capture_candidate` / `apply_patch` / `activate_candidate`.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, JobId, PackageIdentity, PackageName,
};
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::WorkspaceManager;

pub async fn git_available() -> bool {
    let out = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await;
    matches!(out, Ok(o) if o.status.success())
}

pub async fn init_repo(path: &std::path::Path) {
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

pub fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("foo").unwrap(),
    )
}

/// Spin up a manager + workspace on a fresh git repo in `handle_root`.
/// Returns `(manager, workspace_id, handle_dir, job_id, store)`.
pub async fn fresh_workspace(
    handle_root: &std::path::Path,
) -> (
    WorkspaceManager<MockStore>,
    ironmaint_workspace::WorkspaceId,
    JobId,
    MockStore,
) {
    let store = MockStore::new();
    let mgr = WorkspaceManager::new(handle_root.to_path_buf(), store.clone());
    let id = mgr.create(JobId::new()).await.unwrap();
    let handle_dir = handle_root.join(format!("{id}"));
    tokio::fs::create_dir_all(&handle_dir).await.unwrap();
    init_repo(&handle_dir).await;
    let job = JobId::new();
    (mgr, id, job, store)
}

/// A small text patch that flips a single README line. The patch
/// must be deterministic so two applications produce the same diff.
pub fn readme_v2_patch() -> String {
    "diff --git a/README.md b/README.md\n\
     index 0000001..0000002 100644\n\
     --- a/README.md\n\
     +++ b/README.md\n\
     @@ -1 +1 @@\n\
     -# hello\n\
     +# hello v2\n"
        .to_string()
}

/// A second patch to drive C2 → C3.
pub fn readme_v3_patch() -> String {
    "diff --git a/README.md b/README.md\n\
     index 0000002..0000003 100644\n\
     --- a/README.md\n\
     +++ b/README.md\n\
     @@ -1 +1 @@\n\
     -# hello v2\n\
     +# hello v3\n"
        .to_string()
}
