//! `assert_executor_conformance` — the §92 exit-checkpoint suite
//! (PHASE-0B.md §92, §47).
//!
//! Each `assert_*` function below exercises one §92 condition
//! against an executor that has the fixture tools registered. The
//! fixture binary is `ironmaint-fixture`; tests register it via
//! [`register_fixture_tools`] (per-tool `ExecutionLimits` tuned
//! for the condition being tested), construct a
//! `ProcessExecutor`, then call [`assert_executor_conformance`].
//!
//! ## What the suite covers
//!
//! - `PASS` — `validate` returns exit_code 0, `truncated=false`,
//!   `outcome_from_record` yields `Outcome::Pass`.
//! - `FAIL` — `fail` returns exit_code 1, `Outcome::Fail`.
//! - `TIMEOUT` — `timeout` (registered with a tight 500ms cap)
//!   surfaces as `ExecutorErrorKind::ToolFailed { timed_out: true }`.
//! - `TRUNCATED` — `truncate` (stdout cap 4 KiB, fixture emits 256
//!   KiB) returns `truncated=true` and `stdout.len() == 4 KiB`.
//!   The persisted `Evidence::with_truncated(true)` round-trips
//!   through JSON with the boolean intact.
//! - `INTERRUPT` — `interrupt` returns exit_code 130,
//!   `Outcome::Interrupted`.
//! - `INFRASTRUCTURE_FAIL` — `infra_fail` returns exit_code 127,
//!   `Outcome::InfrastructureFailed`.
//!
//! `truncated = true` is orthogonal to outcome classification: a
//! truncated PASS is still `Pass` (PHASE-0B.md §15 last bullet).
//!
//! ## Distribution neutrality
//!
//! This module does not import `ironmaint-evidence` adapter types
//! or any distribution identifier. The only "Debian"-shaped
//! string in the test surface is the fixture's tool_key, which
//! is a synthetic `synthetic.*` namespace from the fixture binary
//! itself.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::CandidateFingerprint;
use ironmaint_evidence::{Evidence, EvidenceKind, EvidenceProducer, EvidenceScope, EvidenceStatus};
use ironmaint_executor::{
    ExecutionRequest, Executor, FixtureNormalizer, Outcome, ResultNormalizer, RetryClass,
    ToolRegistry, outcome_from_record,
};
use serde_json::json;
use time::macros::datetime;

/// Pre-registered tool keys returned from
/// [`register_fixture_tools`]. Tests pass this to
/// [`assert_executor_conformance`] so the suite exercises the
/// same fixture binary the harness uses.
///
/// The definitions themselves live in `ironmaint-synthetic-tools`
/// (Phase 0B.9) so the daemon can register the same set without
/// taking a production dependency on this crate, which
/// PHASE-0A.md §67 forbids.
pub type FixtureKeys = ironmaint_synthetic_tools::SyntheticKeys;

/// Register the six fixture tool keys against `registry`, pointing
/// at `fixture_bin`.
///
/// Thin delegate to
/// [`ironmaint_synthetic_tools::register_synthetic_tools`] so the
/// conformance runner and the daemon share one source of truth for
/// the synthetic tool set.
///
/// # Panics
/// Panics if a key fails `ToolCapabilityKey::new` or the registry
/// rejects a record. The test harness owns its registry, so that is
/// a harness bug rather than a runtime condition; production
/// callers (the daemon) use the fallible form directly.
pub fn register_fixture_tools(registry: &mut ToolRegistry, fixture_bin: &Path) -> FixtureKeys {
    ironmaint_synthetic_tools::register_synthetic_tools(registry, fixture_bin)
        .expect("synthetic tool registration")
}

/// Run the full §92 exit-checkpoint suite against `executor`.
/// `keys` must come from [`register_fixture_tools`] so the
/// executor's registry knows the six tool keys exercised here.
///
/// # Panics
/// Panics on the first contract violation. Each `assert_*`
/// helper pinpoints the failing condition via a descriptive
/// message.
pub async fn assert_executor_conformance<E: Executor + ?Sized>(executor: &E, keys: &FixtureKeys) {
    assert_pass(executor, &keys.validate).await;
    assert_fail(executor, &keys.fail).await;
    assert_timeout(executor, &keys.timeout).await;
    assert_truncate(executor, &keys.truncate).await;
    assert_interrupt(executor, &keys.interrupt).await;
    assert_infra_fail(executor, &keys.infra_fail).await;
    assert_normalizer_round_trip();
    assert_evidence_truncated_round_trip();
}

// -----------------------------------------------------------------------------
// §92 conditions.
// -----------------------------------------------------------------------------

/// A job id for an execution.
///
/// The executor's artifact guard is per-job (§15), so a request has
/// to name one; the id itself is not what any of these tests are
/// about.
fn job_id() -> ironmaint_core::JobId {
    ironmaint_core::JobId::new()
}

async fn assert_pass<E: Executor + ?Sized>(executor: &E, key: &ToolCapabilityKey) {
    let rec = executor
        .execute(ExecutionRequest::new(
            job_id(),
            key.clone(),
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("§92 PASS must produce an ExecutionRecord");
    assert_eq!(rec.exit_code, 0, "§92 PASS: expected exit_code 0");
    assert!(!rec.truncated, "§92 PASS: expected truncated=false");
    assert_eq!(outcome_from_record(&rec), Outcome::Pass);
}

async fn assert_fail<E: Executor + ?Sized>(executor: &E, key: &ToolCapabilityKey) {
    let rec = executor
        .execute(ExecutionRequest::new(
            job_id(),
            key.clone(),
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("§92 FAIL must produce an ExecutionRecord");
    assert_eq!(rec.exit_code, 1, "§92 FAIL: expected exit_code 1");
    assert_eq!(outcome_from_record(&rec), Outcome::Fail);
}

async fn assert_timeout<E: Executor + ?Sized>(executor: &E, key: &ToolCapabilityKey) {
    let err = executor
        .execute(ExecutionRequest::new(
            job_id(),
            key.clone(),
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect_err("§92 TIMEOUT must surface as ExecutorErrorKind::ToolFailed");
    assert_eq!(
        err.kind,
        ironmaint_executor::ExecutorErrorKind::ToolFailed { timed_out: true },
        "§92 TIMEOUT: expected ToolFailed{{timed_out:true}}, got {:?}",
        err.kind
    );
}

async fn assert_truncate<E: Executor + ?Sized>(executor: &E, key: &ToolCapabilityKey) {
    let rec = executor
        .execute(ExecutionRequest::new(
            job_id(),
            key.clone(),
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("§92 TRUNCATE must produce an ExecutionRecord");
    assert_eq!(
        rec.exit_code, 0,
        "§92 TRUNCATE: expected exit_code 0 (PASS)"
    );
    assert!(
        rec.truncated,
        "§92 TRUNCATE: executor must set truncated=true when stdout exceeds cap"
    );
    assert_eq!(
        rec.stdout.len(),
        4 * 1024,
        "§92 TRUNCATE: bounded stdout must equal cap exactly"
    );
    // Truncation is orthogonal to outcome — a truncated PASS stays
    // Pass (PHASE-0B.md §15 last bullet).
    assert_eq!(outcome_from_record(&rec), Outcome::Pass);
}

async fn assert_interrupt<E: Executor + ?Sized>(executor: &E, key: &ToolCapabilityKey) {
    let rec = executor
        .execute(ExecutionRequest::new(
            job_id(),
            key.clone(),
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("§92 INTERRUPT must produce an ExecutionRecord");
    assert_eq!(
        rec.exit_code, 130,
        "§92 INTERRUPT: expected exit_code 130 (128 + SIGINT)"
    );
    assert_eq!(outcome_from_record(&rec), Outcome::Interrupted);
}

async fn assert_infra_fail<E: Executor + ?Sized>(executor: &E, key: &ToolCapabilityKey) {
    let rec = executor
        .execute(ExecutionRequest::new(
            job_id(),
            key.clone(),
            RetryClass::Safe,
            json!({}),
        ))
        .await
        .expect("§92 INFRASTRUCTURE_FAIL must produce an ExecutionRecord");
    assert_eq!(
        rec.exit_code, 127,
        "§92 INFRASTRUCTURE_FAIL: expected exit_code 127"
    );
    assert_eq!(outcome_from_record(&rec), Outcome::InfrastructureFailed);
}

// -----------------------------------------------------------------------------
// Cross-crate round-trip checks.
// -----------------------------------------------------------------------------

fn assert_normalizer_round_trip() {
    let normalizer = FixtureNormalizer;
    let now = datetime!(2026-01-01 00:00:00 UTC);
    let key = ToolCapabilityKey::new("synthetic.build.validate").unwrap();

    let rec = ironmaint_executor::ExecutionRecord {
        tool_key: key.clone(),
        retry_class: RetryClass::Safe,
        started_at: now,
        finished_at: now,
        exit_code: 0,
        stdout: "BUILD OK\n".to_string(),
        stderr: String::new(),
        retries_exhausted: false,
        truncated: false,

        // Constructed rather than executed: nothing spilled.
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    };
    let n = normalizer
        .normalize(&rec)
        .expect("normalizer accepts the record");
    assert_eq!(n.evidence_status, EvidenceStatus::Pass);
    assert!(!n.output_truncated);
    assert!(n.invalidations.is_empty());

    let rec_fail = ironmaint_executor::ExecutionRecord {
        tool_key: key,
        retry_class: RetryClass::Safe,
        started_at: now,
        finished_at: now,
        exit_code: 1,
        stdout: "build failed\n".to_string(),
        stderr: String::new(),
        retries_exhausted: false,
        truncated: false,

        // Constructed rather than executed: nothing spilled.
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    };
    let n_fail = normalizer
        .normalize(&rec_fail)
        .expect("normalizer accepts failure record");
    assert_eq!(n_fail.evidence_status, EvidenceStatus::Fail);
    assert_eq!(n_fail.observations.len(), 1);
    assert_eq!(
        n_fail.invalidations,
        vec!["fixture_exit_nonzero".to_string()]
    );
}

fn assert_evidence_truncated_round_trip() {
    // PHASE-0B.md §15: evidence rows must explicitly record
    // `truncated = true`. Confirm the builder + serde path
    // preserves the boolean.
    let fp = CandidateFingerprint::from_hex("a".repeat(64)).unwrap();
    let base = Evidence::new(
        fp.clone(),
        EvidenceKind::Other("tool_output".to_string()),
        EvidenceStatus::Pass,
        EvidenceProducer::new("ironmaint-fixture"),
        EvidenceScope::Candidate(fp),
        datetime!(2026-01-01 00:00:00 UTC),
    );

    let truncated = base.clone().with_truncated(true);
    assert!(truncated.truncated);
    let json = serde_json::to_string(&truncated).expect("serialise");
    let back: Evidence = serde_json::from_str(&json).expect("parse");
    assert!(back.truncated);

    let not_truncated = base.with_truncated(false);
    let json = serde_json::to_string(&not_truncated).expect("serialise");
    let back: Evidence = serde_json::from_str(&json).expect("parse");
    assert!(!back.truncated);
}

/// Resolve the `ironmaint-fixture` binary path. Mirrors the
/// pattern in `crates/ironmaint-executor/tests/process_exit_codes.rs`
/// — cross-crate tests cannot use `CARGO_BIN_EXE_<name>`, so the
/// path is resolved relative to the workspace target dir.
pub fn fixture_binary_path() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolves from testkit/../..");
    let mut path = workspace_dir
        .join("target")
        .join("debug")
        .join("ironmaint-fixture");
    if !path.exists() {
        path = workspace_dir
            .join("target")
            .join("release")
            .join("ironmaint-fixture");
    }
    path
}
