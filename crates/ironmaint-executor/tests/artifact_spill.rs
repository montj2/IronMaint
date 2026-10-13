//! D-08 — `ProcessExecutor` retains the output it bounded.
//!
//! `ProcessExecutor` held an `ArtifactStore` and a
//! `JobArtifactGuardFactory` and used neither. A tool that printed
//! more than `stdout_max_bytes` had the excess drained and
//! discarded, `ExecutionRecord.truncated` was set, and nothing
//! else happened: no artifact, no error, no indication beyond the
//! boolean that the log an operator wanted was gone.
//!
//! Worse, `bins/ironmaintd` opened the store and logged
//! "artifact store verified" while nothing was ever written to it.
//!
//! These drive the real `ironmaint-fixture` binary through the real
//! executor, because the defect is only observable at the seam: the
//! reader, the store, the guard and the record are each correct
//! alone. The claim is not "the executor can write to a store" —
//! that was always true. It is "the bytes the record had to leave
//! behind are somewhere, named, and bounded".
// `panic!` is allowed alongside `unwrap`/`expect` for the same reason:
// a failing test should stop, and a message that names what was
// actually present is worth more than a bare `None`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_core::JobId;
use ironmaint_executor::artifact_guard::JobArtifactGuard;
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRequest, Executor, LimitsConfig, ProcessExecutor,
    RetryClass, ToolDefinitionRecord, ToolRegistry, factory_from_config,
};
use serde_json::json;

// -----------------------------------------------------------------------------
// Harness.
// -----------------------------------------------------------------------------

fn fixture_bin() -> PathBuf {
    ironmaint_fixture::fixture_binary_path()
}

fn temp_store() -> (tempfile::TempDir, Arc<ArtifactStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ArtifactStore::open(ArtifactRoot::new(dir.path())));
    (dir, store)
}

fn register(registry: &mut ToolRegistry, key: &str, limits: ExecutionLimits) -> ToolCapabilityKey {
    let ck = ToolCapabilityKey::new(key).unwrap();
    registry
        .register(Box::new(ToolDefinitionRecord::new(
            ck.clone(),
            fixture_bin(),
            vec![OsString::from(key)],
            ExecutionClass::Check,
            limits,
        )))
        .expect("register");
    ck
}

fn limits(stdout_max: u64) -> ExecutionLimits {
    ExecutionLimits {
        timeout: std::time::Duration::from_secs(30),
        stdout_max_bytes: stdout_max,
        stderr_max_bytes: 64 * 1024,
    }
}

fn executor(registry: Arc<ToolRegistry>, store: Arc<ArtifactStore>) -> ProcessExecutor {
    ProcessExecutor::new(registry, store, Default::default()).with_time_factory(Arc::new(|| {
        time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }))
}

async fn run(
    exec: &ProcessExecutor,
    job: JobId,
    key: &ToolCapabilityKey,
) -> ironmaint_executor::ExecutionRecord {
    exec.execute(ExecutionRequest::new(
        job,
        key.clone(),
        RetryClass::Safe,
        json!({}),
    ))
    .await
    .expect("execute")
}

fn artifact_for(
    rec: &ironmaint_executor::ExecutionRecord,
    stream: ironmaint_executor::OutputStream,
) -> &ironmaint_executor::SpilledArtifact {
    rec.artifacts
        .iter()
        .find(|a| a.stream == stream)
        .unwrap_or_else(|| {
            panic!(
                "expected a {stream:?} artifact; the record names {:?} \
                 and dropped {:?}",
                rec.artifacts, rec.artifacts_dropped
            )
        })
}

// -----------------------------------------------------------------------------
// The defect.
// -----------------------------------------------------------------------------

/// **The test D-08 needed and did not have.** The record bounds its
/// copy at 8 KiB; the fixture prints 256 KiB. Before the fix the
/// other 248 KiB were drained into `/dev/null`, `truncated` was
/// `true`, and the run was over.
#[tokio::test]
async fn output_beyond_the_record_cap_is_retained_as_an_artifact() {
    let (_dir, store) = temp_store();
    let mut reg = ToolRegistry::new();
    let key = register(&mut reg, "synthetic.build.truncate", limits(8 * 1024));
    let exec = executor(Arc::new(reg), Arc::clone(&store));

    let rec = run(&exec, JobId::new(), &key).await;

    assert!(
        rec.truncated,
        "precondition: the record's own copy is bounded"
    );

    let artifact = artifact_for(&rec, ironmaint_executor::OutputStream::Stdout);
    assert_eq!(
        rec.stdout.len(),
        8 * 1024,
        "the ergonomic copy is still the bounded prefix"
    );
    assert!(
        artifact.bytes > rec.stdout.len() as u64,
        "the artifact must hold more than the record does — that is the \
         whole point. record={} artifact={}",
        rec.stdout.len(),
        artifact.bytes
    );

    // And it must be *readable*. A digest that cannot be resolved
    // is not retention, it is a claim of retention.
    let bytes = store.get(&artifact.digest).await.expect("read back");
    assert_eq!(bytes.len() as u64, artifact.bytes);
    assert!(
        bytes.starts_with(&[b'A'; 16]),
        "and it must be the tool's output, not something else"
    );
}

/// The artifact is the **complete** stream, not the record's
/// prefix, and the record says how much of it it could not keep.
///
/// The two facts are separate and both are load-bearing. Without
/// the first, spilling is the truncation written twice. Without the
/// second, an operator reading a 1 MiB artifact has no way to know
/// the tool printed 40.
#[tokio::test]
async fn the_artifact_is_the_whole_stream_and_says_what_it_could_not_keep() {
    let (dir, store) = temp_store();
    let mut reg = ToolRegistry::new();
    let key = register(&mut reg, "synthetic.build.truncate", limits(8 * 1024));
    // A per-artifact cap well below the fixture's output, so the
    // artifact itself is bounded and `dropped_bytes` is non-zero.
    let exec = executor(Arc::new(reg), Arc::clone(&store)).with_guard_factory(Arc::new(|job| {
        Arc::new(JobArtifactGuard::new(
            job,
            LimitsConfig::default().per_job_max_artifacts,
            32 * 1024,
        ))
    }));

    let rec = run(&exec, JobId::new(), &key).await;
    let artifact = artifact_for(&rec, ironmaint_executor::OutputStream::Stdout);

    assert_eq!(artifact.bytes, 32 * 1024, "the artifact is capped");
    assert!(
        artifact.dropped_bytes > 0,
        "and the record must say the stream was longer: the fixture \
         writes 256 KiB, so {} bytes went unrecorded",
        artifact.dropped_bytes
    );
    assert_eq!(
        artifact.bytes + artifact.dropped_bytes,
        256 * 1024,
        "retained + dropped must reconstruct what the tool actually \
         printed — this is what makes `dropped_bytes` a measurement \
         rather than a flag"
    );

    // The bytes really are on disk at the stated size.
    let path = store.path_for(&artifact.digest);
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        artifact.bytes,
        "the file on disk matches what the record claims"
    );
    drop(dir);
}

/// Both streams are retained, not just the one that overflows. A
/// tool that fails prints its diagnostics to stderr, and stderr is
/// the stream an operator most often wants.
#[tokio::test]
async fn both_streams_are_retained() {
    let (_dir, store) = temp_store();
    let mut reg = ToolRegistry::new();
    let key = register(&mut reg, "synthetic.qa.fail", limits(64 * 1024));
    let exec = executor(Arc::new(reg), Arc::clone(&store));

    let rec = run(&exec, JobId::new(), &key).await;

    for stream in [
        ironmaint_executor::OutputStream::Stdout,
        ironmaint_executor::OutputStream::Stderr,
    ] {
        let a = artifact_for(&rec, stream);
        let bytes = store.get(&a.digest).await.expect("read back");
        assert_eq!(bytes.len() as u64, a.bytes, "{stream:?} round-trips");
    }
}

// -----------------------------------------------------------------------------
// The budget is a refusal, and it is reported.
// -----------------------------------------------------------------------------

/// §15's cap refuses; it does not trim. When a job's artifact
/// budget is spent the executor must say so **on the record**.
///
/// The failure this guards is quiet in a specific way: the run
/// succeeds, the record is complete for everything it claims, and
/// the fact that the build log was not kept is discoverable only by
/// looking at the store and counting.
#[tokio::test]
async fn a_spent_job_budget_is_reported_on_the_record_rather_than_silently_omitting_it() {
    let (_dir, store) = temp_store();
    let mut reg = ToolRegistry::new();
    let key = register(&mut reg, "synthetic.build.truncate", limits(8 * 1024));
    // One artifact allowed. Each run wants two (stdout + stderr),
    // so the second run's stdout must be refused.
    let exec = executor(Arc::new(reg), Arc::clone(&store)).with_guard_factory(Arc::new(|job| {
        Arc::new(JobArtifactGuard::new(job, 1, u64::MAX))
    }));

    let job = JobId::new();
    let first = run(&exec, job, &key).await;
    let second = run(&exec, job, &key).await;

    assert!(
        second
            .artifacts_dropped
            .iter()
            .any(|d| d.reason == "budget_count"),
        "the second run must name the budget as the reason: {:?}",
        second.artifacts_dropped
    );
    // The refusal is a fact about retention, not about the run. A
    // tool that printed its output has still printed it, and the
    // exit code has not changed.
    assert_eq!(first.exit_code, second.exit_code);
    assert_eq!(second.stdout, first.stdout);
}

/// The count cap and the byte cap are different failures and are
/// reported as such. A single "budget" string would not let an
/// operator tell "this job may not keep any more artifacts" from
/// "this job may keep artifacts, just not this much".
#[tokio::test]
async fn the_byte_budget_and_the_count_budget_are_distinguishable() {
    let (_dir, store) = temp_store();
    let mut reg = ToolRegistry::new();
    let key = register(&mut reg, "synthetic.build.truncate", limits(8 * 1024));

    let exhausted = executor(Arc::new(reg), Arc::clone(&store))
        .with_guard_factory(Arc::new(|job| Arc::new(JobArtifactGuard::new(job, 10, 0))));
    let rec = run(&exhausted, JobId::new(), &key).await;
    assert!(
        rec.artifacts_dropped
            .iter()
            .any(|d| d.reason == "budget_bytes"),
        "an exhausted byte budget is not an exhausted count: {:?}",
        rec.artifacts_dropped
    );
    assert!(
        rec.artifacts.is_empty(),
        "and the refusal must not be dressed up as a zero-byte artifact"
    );
}

// -----------------------------------------------------------------------------
// Wire format.
// -----------------------------------------------------------------------------

/// Older records parse: both fields are `#[serde(default)]`, so a
/// payload written before D-08 round-trips and reads as "no
/// artifacts", which is what it meant.
#[test]
fn a_record_written_before_this_change_still_parses() {
    let json = json!({
        "tool_key": "synthetic.build.validate",
        "retry_class": "safe",
        "started_at": "2026-01-01T00:00:00Z",
        "finished_at": "2026-01-01T00:00:01Z",
        "exit_code": 0,
        "stdout": "",
        "stderr": "",
        "retries_exhausted": false,
    })
    .to_string();
    let rec: ironmaint_executor::ExecutionRecord = serde_json::from_str(&json).unwrap();
    assert!(rec.artifacts.is_empty());
    assert!(rec.artifacts_dropped.is_empty());
}

/// The new fields serialise, and the digest survives the round
/// trip as a digest rather than as an opaque string.
#[test]
fn artifacts_round_trip_through_json() {
    let rec = ironmaint_executor::ExecutionRecord {
        tool_key: ToolCapabilityKey::new("synthetic.build.validate").unwrap(),
        retry_class: RetryClass::Safe,
        started_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        finished_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_001).unwrap(),
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
        retries_exhausted: false,
        truncated: true,
        artifacts: vec![ironmaint_executor::SpilledArtifact {
            stream: ironmaint_executor::OutputStream::Stdout,
            digest: ironmaint_artifacts::hash::sha256_of_bytes(b"hello"),
            bytes: 5,
            dropped_bytes: 99,
        }],
        artifacts_dropped: vec![ironmaint_executor::DroppedArtifact {
            stream: ironmaint_executor::OutputStream::Stderr,
            reason: "budget_bytes".to_string(),
        }],
    };

    let back: ironmaint_executor::ExecutionRecord =
        serde_json::from_str(&serde_json::to_string(&rec).unwrap()).unwrap();
    assert_eq!(rec, back);
}

// A `factory_from_config` smoke test, because the daemon now
// depends on it and nothing else in the workspace did.
#[test]
fn the_config_driven_factory_uses_the_configs_caps() {
    let guard = factory_from_config(LimitsConfig {
        per_job_max_artifacts: 3,
        per_job_max_bytes: 64,
        per_artifact_max_bytes: 32,
    })(JobId::new());
    assert_eq!(guard.max_count(), 3);
    assert_eq!(guard.max_bytes(), 64);
}
