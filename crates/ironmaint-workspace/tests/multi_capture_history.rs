//! §91 exit checkpoint: a fixture repository can produce
//! C1 → edit → C2 → edit → C3 with immutable fingerprints and
//! preserved history, then `activate_candidate(C3)`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use ironmaint_core::JobId;
use ironmaint_store::CandidateStore;
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::{WorkspaceId, WorkspaceManager, WorkspaceRevision, git::GitInvocation};

#[tokio::test]
async fn phase_0b_3_exit_checkpoint() {
    if !common::git_available().await {
        eprintln!("git not available; skipping §91 exit-checkpoint test");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let (mgr, id, job, store): (WorkspaceManager<MockStore>, WorkspaceId, JobId, MockStore) =
        common::fresh_workspace(&handle_root).await;

    let ws_dir = handle_root.join(format!("{id}"));

    // C1
    let c1 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    let rev1 = mgr.current_revision(id).await.unwrap();

    // edit → C2
    let rev_after_apply_1 = mgr
        .apply_patch(id, rev1, &common::readme_v2_patch())
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
    let rev2 = rev_after_apply_1;

    // edit → C3
    let rev_after_apply_2 = mgr
        .apply_patch(id, rev2, &common::readme_v3_patch())
        .await
        .unwrap();
    let c3 = mgr
        .capture_candidate(
            id,
            job,
            common::package(),
            "https://example.invalid/foo.git",
        )
        .await
        .unwrap();
    let rev3 = rev_after_apply_2;

    // Immutable fingerprints: all three pairwise distinct.
    assert_ne!(c1.fingerprint(), c2.fingerprint());
    assert_ne!(c2.fingerprint(), c3.fingerprint());
    assert_ne!(c1.fingerprint(), c3.fingerprint());

    // Revisions grew monotonically.
    assert!(
        rev1 < rev2,
        "expected rev1 < rev2, got {rev1:?} vs {rev2:?}"
    );
    assert!(
        rev2 < rev3,
        "expected rev2 < rev3, got {rev2:?} vs {rev3:?}"
    );

    // History preserved: ≥ 3 §22 maintenance commits.
    let inv = GitInvocation::for_maintenance_commit(&ws_dir);
    let log = inv.log(10).await.unwrap();
    let maintenance_count = log
        .iter()
        .filter(|c| c.subject == "IronMaint internal maintenance commit")
        .count();
    assert!(
        maintenance_count >= 3,
        "expected ≥ 3 §22 maintenance commits, got {maintenance_count} (log: {log:?})"
    );

    // Activate C3 — this is the §91 promotion step.
    let activated = mgr.activate_candidate(id, &c3).await.unwrap();
    assert!(
        activated > rev3,
        "activation should bump revision past the pre-activation state, got {activated:?}"
    );

    // Post-activation state: base_candidate == C3, dirty == false.
    let state = mgr.state(id).await.unwrap();
    assert_eq!(state.base_candidate, Some(c3.id()));
    assert!(!state.dirty, "activation must clear the dirty flag");

    // The active_source_candidates row points at C3.
    let active = store.active_source_candidate(job).await.unwrap();
    assert_eq!(
        active,
        Some(c3.id()),
        "active_source_candidates row must point at C3"
    );

    // Sanity: WorkspaceRevision::ZERO < (1) < (2) — three captures
    // with two apply_patches in between means the post-activation
    // revision is 4 (capture bumps nothing, apply bumps by 1 each,
    // activation bumps by 1).
    assert_eq!(rev1, WorkspaceRevision::ZERO);
    assert_eq!(rev2, WorkspaceRevision(1));
    assert_eq!(rev3, WorkspaceRevision(2));
    assert_eq!(activated, WorkspaceRevision(3));
}
