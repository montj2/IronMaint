//! PHASE-2 PR 2A.1 — hermetic tests for
//! `DebianBuildReportNormalizer`.
//!
//! These tests construct canned `ExecutionRecord`s with
//! pre-baked stdout and call the normalizer directly,
//! without spawning a subprocess. The 5 scenarios cover
//! every `BuildOutcome` variant the spec lists in
//! `doc/phases/PHASE-2.md` §82–96:
//!
//! 1. `Pass` — normalises to `EvidenceStatus::Pass`.
//! 2. `PackageBuildFailed` — normalises to
//!    `EvidenceStatus::Fail` with diagnostics as
//!    observations.
//! 3. `InfrastructureFailed` — normalises to
//!    `Err(NormalizationError::Unclassifiable)`.
//! 4. `Malformed` stdout — normalises to
//!    `Err(NormalizationError::Malformed)`.
//! 5. `MissingTool` (empty stdout, exit 127) —
//!    normalises to
//!    `Err(NormalizationError::Unclassifiable)`.
//!
//! RED: this file imports `DebianBuildReportNormalizer`,
//! `DebianBuildReportV1`, and `BuildOutcome` which do not
//! exist in the GREEN-less workspace. The
//! `cargo test -p ironmaint-debian-tool` step fails to
//! compile this file; that compile failure is the RED
//! state. Once the GREEN commit lands the types and the
//! normalizer, the tests compile and pass.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_debian_tool::{BuildOutcome, DebianBuildReportNormalizer, DebianBuildReportV1};
use ironmaint_evidence::EvidenceStatus;
use ironmaint_executor::{ExecutionRecord, NormalizationError, ResultNormalizer, RetryClass};
use time::macros::datetime;

/// Build a minimal `ExecutionRecord` for normalizer
/// tests. The normalizer reads only `stdout` (to parse
/// the report) and `truncated` (to propagate into
/// `NormalizedResult.output_truncated`); the other
/// fields are pinned to representative defaults so the
/// test fixtures are stable.
fn make_record(stdout: &str, exit_code: i32) -> ExecutionRecord {
    ExecutionRecord {
        tool_key: ToolCapabilityKey::new("debian.build.sbuild").unwrap(),
        retry_class: RetryClass::Safe,
        started_at: datetime!(2026-01-01 00:00:00 UTC),
        finished_at: datetime!(2026-01-01 00:00:01 UTC),
        exit_code,
        stdout: stdout.to_string(),
        stderr: String::new(),
        retries_exhausted: false,
        truncated: false,
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    }
}

/// Build a minimal `DebianBuildReportV1` for the
/// `Pass` case. The 2A.1 stub fills only the
/// well-typed fields the normalizer reads; the rest
/// are `Default::default()`.
fn pass_report() -> DebianBuildReportV1 {
    DebianBuildReportV1 {
        outcome: BuildOutcome::Pass,
        ..Default::default()
    }
}

/// A `BuildOutcome::Pass` report normalises to
/// `EvidenceStatus::Pass` with no observations.
#[test]
fn build_pass_report_normalises_to_pass() {
    let report = pass_report();
    let json = serde_json::to_string(&report).unwrap();
    let result = DebianBuildReportNormalizer::new()
        .normalize(&make_record(&json, 0))
        .unwrap();
    assert_eq!(result.evidence_status, EvidenceStatus::Pass);
    assert!(result.observations.is_empty());
    assert!(result.invalidations.is_empty());
    assert!(!result.output_truncated);
}

/// A `BuildOutcome::PackageBuildFailed` report
/// normalises to `EvidenceStatus::Fail` with the
/// report's diagnostics propagated as observations.
#[test]
fn build_package_fail_report_normalises_to_fail() {
    let report = DebianBuildReportV1 {
        outcome: BuildOutcome::PackageBuildFailed,
        diagnostics: vec![ironmaint_debian_tool::Diagnostic {
            code: "E_BUILD_RULES_FAILED".to_string(),
            message: "debian/rules build returned exit 2".to_string(),
            path: Some(std::path::PathBuf::from("debian/rules")),
        }],
        ..Default::default()
    };
    let json = serde_json::to_string(&report).unwrap();
    let result = DebianBuildReportNormalizer::new()
        .normalize(&make_record(&json, 2))
        .unwrap();
    assert_eq!(result.evidence_status, EvidenceStatus::Fail);
    assert_eq!(result.observations.len(), 1);
    assert_eq!(result.observations[0].kind, "E_BUILD_RULES_FAILED");
    assert!(
        result.observations[0]
            .message
            .contains("debian/rules build returned exit 2")
    );
    assert!(result.invalidations.is_empty());
}

/// A `BuildOutcome::InfrastructureFailed` report
/// normalises to `Err(NormalizationError::Unclassifiable(...))`.
/// The runtime's normalizer-error path turns this into
/// `ExecutorError::InfrastructureFailed` →
/// `GateStatus::Blocked`.
#[test]
fn build_infra_fail_report_normalises_to_unclassifiable() {
    let report = DebianBuildReportV1 {
        outcome: BuildOutcome::InfrastructureFailed,
        diagnostics: vec![ironmaint_debian_tool::Diagnostic {
            code: "E_INFRA_SCHROOT".to_string(),
            message: "schroot chroot not found at /var/lib/sbuild/sid-amd64".to_string(),
            path: None,
        }],
        ..Default::default()
    };
    let json = serde_json::to_string(&report).unwrap();
    let err = DebianBuildReportNormalizer::new()
        .normalize(&make_record(&json, 0))
        .unwrap_err();
    assert!(matches!(err, NormalizationError::Unclassifiable(_)));
}

/// A stdout that is not valid JSON is a `Malformed`
/// error (the runtime turns this into `Outcome::Fail`
/// — a defect, not an outage). The spec's "Unknown
/// failure modes must not be silently classified as
/// package defects" requirement is met by the
/// `Unknown` variant's `Unclassifiable` mapping; this
/// `Malformed` mapping is the dedicated "tool produced
/// garbage" arm.
#[test]
fn build_malformed_stdout_normalises_to_malformed() {
    let err = DebianBuildReportNormalizer::new()
        .normalize(&make_record("not valid json", 0))
        .unwrap_err();
    assert!(matches!(err, NormalizationError::Malformed(_)));
}

/// A `MissingTool` scenario (empty stdout, exit 127
/// — POSIX "command not found") normalises to
/// `Err(NormalizationError::Unclassifiable(...))`. The
/// runtime's normalizer-error path turns this into
/// `ExecutorError::InfrastructureFailed` →
/// `GateStatus::Blocked`.
#[test]
fn build_missing_tool_normalises_to_unclassifiable() {
    let err = DebianBuildReportNormalizer::new()
        .normalize(&make_record("", 127))
        .unwrap_err();
    assert!(matches!(err, NormalizationError::Unclassifiable(_)));
}
