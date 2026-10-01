//! The three workspace-backed MCP tools, exercised over real git.
//!
//! `workspace.stat`, `workspace.apply_patch`, and
//! `candidate.capture` are the tools that were shape-only in 0B.6:
//! `stat` returned a hardcoded `WorkspaceRevision::new()`,
//! `apply_patch` echoed `expected_revision` back unchanged, and
//! `candidate.capture` fabricated `"0".repeat(40)` for both the
//! commit and the tree OID.
//!
//! The fabricated hashes were not merely incomplete — they were
//! silently wrong. A `SourceCandidate`'s fingerprint is derived
//! from its commit and tree, so every capture of a given package
//! collided on one fingerprint, and since
//! `RuntimeCommand::CaptureCandidate` deduplicates by fingerprint,
//! a genuine upstream source update was indistinguishable from a
//! repeat of the previous capture. `capture_reflects_source_state`
//! and `capture_is_deterministic` below are the regression tests
//! for that.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, JobId, PackageIdentity, PackageName,
};
use ironmaint_executor::{
    ExecutionRecord, ExecutionRequest, Executor, ExecutorError, RetryClass, ToolRegistry,
};
use ironmaint_mcp::schema::McpToolName;
use ironmaint_mcp::{McpError, McpRuntime, dispatch};
use ironmaint_runtime::{RuntimeService, SystemClock};
use ironmaint_store::CandidateStore;
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::WorkspaceManager;
use time::OffsetDateTime;

#[derive(Clone)]
struct PassExecutor;

impl Executor for PassExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .unwrap_or_else(|_| OffsetDateTime::now_utc());
        Ok(ExecutionRecord {
            tool_key: request.tool_key.clone(),
            retry_class: RetryClass::Safe,
            started_at: now,
            finished_at: now,
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            retries_exhausted: false,
            truncated: false,

            // Constructed rather than executed: nothing spilled.
            artifacts: Vec::new(),
            artifacts_dropped: Vec::new(),
        })
    }
}

/// A dispatcher wired to a real git working tree, plus the store
/// handle the assertions need. The workspace manager and the
/// runtime share one `Arc<MockStore>` — the daemon's arrangement.
struct Harness {
    mcp: McpRuntime<MockStore, PassExecutor>,
    store: Arc<MockStore>,
    _tmp: tempfile::TempDir,
}

fn harness() -> Harness {
    let tmp = tempfile::tempdir().expect("temp workspace root");
    let store = Arc::new(MockStore::new());
    let clock: Arc<dyn ironmaint_runtime::Clock> = Arc::new(SystemClock);
    let executor = Arc::new(PassExecutor);
    let registry = Arc::new(ToolRegistry::new());
    let service = Arc::new(RuntimeService::new(
        Arc::clone(&store),
        clock,
        executor,
        registry,
    ));
    let workspace = Arc::new(WorkspaceManager::new(
        tmp.path().to_path_buf(),
        Arc::clone(&store),
    ));
    Harness {
        mcp: McpRuntime::new(service).with_workspace(workspace),
        store,
        _tmp: tmp,
    }
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").expect("family"),
            DistributionRelease::new("unstable").expect("release"),
        ),
        PackageName::new("fixture-pkg").expect("name"),
    )
}

/// Create a job over MCP and return its id. No direct store
/// access — this is the §94 "no direct store access" discipline.
async fn create_job(h: &Harness) -> JobId {
    let out = dispatch(
        h.mcp.clone(),
        &McpToolName("job.create".to_string()),
        serde_json::json!({
            "orchestrator": { "kind": "ironclaw" },
            "package": {
                "distribution": { "family": "debian", "release": "sid" },
                "source_name": "fixture-pkg",
                "binary_names": [],
            },
        }),
    )
    .await
    .expect("job.create must succeed");
    serde_json::from_value(out.get("job_id").cloned().expect("job_id present"))
        .expect("job_id deserializable")
}

async fn stat(h: &Harness, job_id: JobId) -> u64 {
    let out = dispatch(
        h.mcp.clone(),
        &McpToolName("workspace.stat".to_string()),
        serde_json::json!({ "job_id": job_id }),
    )
    .await
    .expect("workspace.stat must succeed");
    out.get("revision")
        .and_then(serde_json::Value::as_u64)
        .expect("revision present")
}

async fn apply_patch(h: &Harness, job_id: JobId, expected: u64, patch: &str) -> serde_json::Value {
    dispatch(
        h.mcp.clone(),
        &McpToolName("workspace.apply_patch".to_string()),
        serde_json::json!({ "job_id": job_id, "expected_revision": expected, "patch": patch }),
    )
    .await
    .expect("workspace.apply_patch must succeed")
}

async fn capture(h: &Harness, job_id: JobId) -> String {
    let out = dispatch(
        h.mcp.clone(),
        &McpToolName("candidate.capture".to_string()),
        serde_json::json!({
            "job_id": job_id,
            "package": package(),
            "repository_url": "https://example.invalid/foo.git",
        }),
    )
    .await
    .expect("candidate.capture must succeed");
    out.get("fingerprint")
        .and_then(serde_json::Value::as_str)
        .expect("fingerprint present")
        .to_string()
}

const PATCH: &str = "\
diff --git a/README.md b/README.md
new file mode 100644
index 0000000..5b0e1f2
--- /dev/null
+++ b/README.md
@@ -0,0 +1 @@
+hello
";

/// A second, different edit, so a capture that accepted it cannot be
/// passing because the tree happened to be unchanged.
const SECOND_PATCH: &str = "\
diff --git a/CHANGELOG.md b/CHANGELOG.md
new file mode 100644
index 0000000..1f0a3c7
--- /dev/null
+++ b/CHANGELOG.md
@@ -0,0 +1 @@
+synthetic package (1.0.1)
";

#[tokio::test]
async fn stat_provisions_a_workspace_and_reports_the_stored_revision() {
    let h = harness();
    let job_id = create_job(&h).await;

    // A freshly-provisioned workspace has never been mutated, so 0
    // is the honest answer — and it is now read from the store
    // rather than fabricated.
    assert_eq!(stat(&h, job_id).await, 0);
    // Idempotent: a second call resolves the same workspace
    // rather than creating a second one, and still reports 0.
    assert_eq!(stat(&h, job_id).await, 0);
}

#[tokio::test]
async fn apply_patch_bumps_the_revision() {
    let h = harness();
    let job_id = create_job(&h).await;
    assert_eq!(stat(&h, job_id).await, 0);

    let out = apply_patch(&h, job_id, 0, PATCH).await;
    assert_eq!(
        out.get("new_revision").and_then(serde_json::Value::as_u64),
        Some(1),
        "apply_patch must report the bumped revision: {out}"
    );
    // The bump is durable, not just returned: re-reading state
    // shows it. The 0B.6 stub echoed `expected_revision` back and
    // wrote nothing.
    assert_eq!(stat(&h, job_id).await, 1);
}

#[tokio::test]
async fn apply_patch_rejects_a_stale_revision_with_a_conflict() {
    let h = harness();
    let job_id = create_job(&h).await;
    apply_patch(&h, job_id, 0, PATCH).await;

    let err = dispatch(
        h.mcp.clone(),
        &McpToolName("workspace.apply_patch".to_string()),
        serde_json::json!({ "job_id": job_id, "expected_revision": 99, "patch": PATCH }),
    )
    .await
    .expect_err("a stale expected_revision must be rejected");
    assert!(
        matches!(err, McpError::Conflict(_)),
        "a stale revision must be a Conflict so the client knows to re-read and retry, got {err:?}"
    );
    // The rejected call must not have moved the revision.
    assert_eq!(stat(&h, job_id).await, 1);
}

/// The regression test for §43's last pipeline step going
/// unimplemented.
///
/// §43 closes `candidate.capture` with "mark workspace clean", and
/// until the dispatch did that, `WorkspaceManager::activate_candidate`
/// had exactly one caller — a workspace test. `apply_patch` refuses a
/// dirty tree, so an agent could patch a job once and never again,
/// with an error naming a cause it had no tool to clear. §101 needs
/// three patches; the second one failed.
///
/// `apply_patch_is_refused_while_the_workspace_is_dirty` below pins
/// the other half of the same rule — and it is *this* test that
/// gives it a boundary: a capture in between is what clears the flag.
#[tokio::test]
async fn capture_marks_the_workspace_clean_so_the_next_patch_is_accepted() {
    let h = harness();
    let job_id = create_job(&h).await;

    apply_patch(&h, job_id, 0, PATCH).await;
    let first = capture(&h, job_id).await;

    // Revision 1 after the first patch, plus one more for the
    // activation the capture performs.
    let after_capture = stat(&h, job_id).await;
    assert!(
        after_capture > 1,
        "capture must record the activation as a distinct revision, got {after_capture}"
    );

    // The point of the test: a second patch, which the dirty flag
    // used to refuse.
    let out = apply_patch(&h, job_id, after_capture, SECOND_PATCH).await;
    assert_eq!(
        out.get("new_revision").and_then(serde_json::Value::as_u64),
        Some(after_capture + 1),
        "the second patch must be accepted and bump the revision: {out}"
    );

    // And the second capture is a genuinely different source, so
    // the repair loop §101 describes can run more than one turn.
    let second = capture(&h, job_id).await;
    assert_ne!(
        first, second,
        "a second patch must produce a second candidate"
    );
}

#[tokio::test]
async fn apply_patch_is_refused_while_the_workspace_is_dirty() {
    let h = harness();
    let job_id = create_job(&h).await;
    apply_patch(&h, job_id, 0, PATCH).await;

    let err = dispatch(
        h.mcp.clone(),
        &McpToolName("workspace.apply_patch".to_string()),
        serde_json::json!({ "job_id": job_id, "expected_revision": 1, "patch": PATCH }),
    )
    .await
    .expect_err("a dirty workspace must refuse a second patch");
    let message = err.to_string();
    assert!(
        message.contains("dirty"),
        "the refusal must say why, got {message}"
    );
}

#[tokio::test]
async fn capture_reflects_source_state() {
    let h = harness();
    let job_id = create_job(&h).await;

    // Capture the empty tree.
    let before = capture(&h, job_id).await;

    // Change the tree, then capture again. The revision is read
    // rather than assumed: a capture now records its activation as a
    // distinct revision, so it is no longer left where it found it.
    apply_patch(&h, job_id, stat(&h, job_id).await, PATCH).await;
    let after = capture(&h, job_id).await;

    // This is the regression test for the 0B.6 stub, which
    // hardcoded `"0".repeat(40)` for both the commit and the tree
    // OID. Two captures of two different trees then produced the
    // same fingerprint, and because the runtime deduplicates
    // captures by fingerprint, the second one resolved to the
    // first candidate — a source update became invisible.
    assert_ne!(
        before, after,
        "a changed working tree must produce a different fingerprint"
    );
}

#[tokio::test]
async fn capture_is_deterministic_for_an_unchanged_tree() {
    let h = harness();
    let job_id = create_job(&h).await;

    let first = capture(&h, job_id).await;
    // `capture_candidate` writes a fresh §22 maintenance commit
    // each time, so the two captures have different HEADs — but the
    // fingerprint is computed over the *pre-commit* commit and the
    // tree OID, which are unchanged. §91 exit checkpoint: "two
    // captures of the same tree must produce one fingerprint".
    let second = capture(&h, job_id).await;
    assert_eq!(
        first, second,
        "re-capturing an unchanged tree must not mint a new fingerprint"
    );
}

#[tokio::test]
async fn capture_writes_exactly_one_candidate_row() {
    let h = harness();
    let job_id = create_job(&h).await;

    let fingerprint = capture(&h, job_id).await;

    // The workspace manager persists the candidate in order to mint
    // its `CandidateId` for the maintenance commit, and the
    // runtime's `CaptureCandidate` handler then looks it up by
    // fingerprint and reuses the id. This pins that the two writes
    // do not produce two rows.
    let ids = h
        .store
        .list_source_candidates_for_job(job_id)
        .await
        .expect("list candidates");
    assert_eq!(ids.len(), 1, "exactly one candidate row expected");

    let stored = h
        .store
        .get_source_candidate(ids[0])
        .await
        .expect("candidate row readable");
    assert_eq!(stored.fingerprint().to_string(), fingerprint);
    assert_eq!(stored.job_id(), job_id);
    // The commit and tree must be real git OIDs, not the stub's
    // forty zeroes.
    assert_ne!(
        stored.commit().as_str(),
        "0000000000000000000000000000000000000000",
        "commit OID must come from the repository, not the stub"
    );
    assert_ne!(
        stored.tree().as_str(),
        "0000000000000000000000000000000000000000",
        "tree OID must come from the working tree, not the stub"
    );
}
