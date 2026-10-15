//! Shared fixture for the workspace integration tests
//! (`capture_candidate`, `apply_patch`, `multi_capture_history`).
//!
//! Each test that needs a real git workspace runs `init_repo`
//! inside a tempdir handle directory, then drives the manager
//! through `capture_candidate` / `apply_patch` / `activate_candidate`.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, JobId, JobProjection, JobState,
    MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName,
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
    // PHASE-1.1 PR 1.1.F: `MockStore::put_source_candidate` now
    // mirrors the SQLite `FOREIGN KEY (job_id) REFERENCES
    // projections(job_id)`, so any test that captures a candidate
    // needs a job projection in place. The runtime's `job.create`
    // flow does this; tests do it once, here, instead of in every
    // capture_candidate case.
    use ironmaint_store::ProjectionStore;
    use time::macros::datetime;
    let now = datetime!(2026-01-01 00:00:00 UTC);
    let proj = JobProjection {
        job: MaintenanceJob::new(job, package(), MaintenanceEventId::new(), now),
        state: JobState::Intake,
        active_candidate: None,
        version: 0,
        updated_at: now,
    };
    store
        .put_projection(&proj, 0)
        .await
        .expect("seed the test job's projection");
    (mgr, id, job, store)
}

/// A small text patch that flips a single README line. Uses
/// the minimal unified-diff format (no `diff --git` or
/// `index` lines with placeholder hashes) so `git apply` can
/// match purely on context.
pub fn readme_v2_patch() -> String {
    "--- a/README.md\n\
     +++ b/README.md\n\
     @@ -1 +1 @@\n\
     -# hello\n\
     +# hello v2\n"
        .to_string()
}

/// A second patch to drive C2 → C3.
pub fn readme_v3_patch() -> String {
    "--- a/README.md\n\
     +++ b/README.md\n\
     @@ -1 +1 @@\n\
     -# hello v2\n\
     +# hello v3\n"
        .to_string()
}
