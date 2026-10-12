//! Unit tests for the 1E.2 `debian.issue.bts_snapshot` subcommand.
//!
//! PHASE-1.md §23, §24, §25. The hermetic mode of the
//! binary's `bts-snapshot` subcommand emits a hard-coded
//! `DebianIssueReportV1` regardless of which source package
//! the candidate names — the binary is hermetic; the live
//! mode (which would shell out to the 1E.1 `bts-helper.py`
//! shim) is opt-in and is not in the §97 gate.
//!
//! ## RED / GREEN state
//!
//! The 1E.2 RED commit's struct is the *minimal* shape
//! (just `schema_version`, `report_kind`, `verdict`); the
//! 1E.2 GREEN commit's struct extends the same shape with
//! the §23 comprehensive fields (`source_package`,
//! `snapshot_at`, `bugs`, `summary`).
//!
//! These tests cover the RED-state struct shape
//! (`verdict: pass`, `report_kind` discriminator,
//! `schema_version` literal) and the round-trip
//! property. The GREEN-state extension is a separate
//! set of tests that assert the comprehensive fields
//! are populated; they live in the same file and fail
//! in the RED commit when those fields are absent.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_debian_tool::DebianBtsSnapshotNormalizer;
use ironmaint_debian_tool::bts_snapshot::{CandidateInput, Package, run};
use ironmaint_debian_tool::report::{
    BTS_SNAPSHOT_REPORT_KIND, BTS_SNAPSHOT_SCHEMA_VERSION, BugSummary, DebianIssueReportV1,
    IssueVerdict,
};
use ironmaint_executor::RetryClass;
use ironmaint_executor::{ExecutionRecord, ResultNormalizer};
use time::macros::datetime;

fn good_input() -> CandidateInput {
    CandidateInput {
        schema: "ironmaint.candidate_input.v3".to_string(),
        job_id: "0192…".to_string(),
        check_id: "0192…".to_string(),
        package: Package {
            name: "example".to_string(),
            version: "1.0.0-1".to_string(),
        },
    }
}

fn make_record(stdout: &str) -> ExecutionRecord {
    ExecutionRecord {
        tool_key: ToolCapabilityKey::new("debian.issue.bts_snapshot").unwrap(),
        retry_class: RetryClass::Safe,
        started_at: datetime!(2026-01-01 00:00:00 UTC),
        finished_at: datetime!(2026-01-01 00:00:01 UTC),
        exit_code: 0,
        stdout: stdout.to_string(),
        stderr: String::new(),
        retries_exhausted: false,
        truncated: false,
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    }
}

/// The hermetic fixture emits a `verdict: pass` report
/// regardless of the candidate's source package. §24
/// "A package having 50 open bugs is not a tool
/// failure" — the snapshot is binary (pass or
/// infrastructure_error), so a populated bug list is
/// never a `Fail`.
#[test]
fn hermetic_fixture_emits_pass_verdict() {
    let report = run(&good_input());
    assert_eq!(report.verdict, IssueVerdict::Pass);
    assert_eq!(report.report_kind, BTS_SNAPSHOT_REPORT_KIND);
    assert_eq!(report.schema_version, BTS_SNAPSHOT_SCHEMA_VERSION);
}

/// The hermetic fixture's discriminator is the §24
/// wire-format literal. A drift here is a wire-format
/// regression: the report would still serialise, but
/// downstream readers would reject the discriminator.
#[test]
fn hermetic_report_kind_discriminator_is_stable() {
    let report = run(&good_input());
    assert_eq!(report.report_kind, "debian.issue.bts_snapshot_v1");
}

/// The comprehensive 1E.2 report round-trips through
/// the wire format without losing data. The 1E.1
/// `bts-helper.py` shim produces a JSON document the
/// normalizer parses; a round-trip through `serde_json`
/// is the structural-equality test.
#[test]
fn comprehensive_report_round_trips_through_json() {
    let report = DebianIssueReportV1 {
        schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
        report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
        verdict: IssueVerdict::Pass,
        source_package: "example".to_string(),
        snapshot_at: "2026-10-09T00:00:00Z".to_string(),
        bugs: Vec::new(),
        summary: BugSummary::default(),
    };
    let json = serde_json::to_string(&report).unwrap();
    let round: DebianIssueReportV1 = serde_json::from_str(&json).unwrap();
    assert_eq!(round, report);
}

/// The normalizer maps `verdict: pass` to
/// `EvidenceStatus::Pass` (the §24 "50 open bugs is
/// not a tool failure" mapping).
#[test]
fn normalizer_maps_pass_verdict_to_pass() {
    let report = DebianIssueReportV1 {
        schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
        report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
        verdict: IssueVerdict::Pass,
        source_package: "example".to_string(),
        snapshot_at: "2026-10-09T00:00:00Z".to_string(),
        bugs: Vec::new(),
        summary: BugSummary::default(),
    };
    let json = serde_json::to_string(&report).unwrap();
    let result = DebianBtsSnapshotNormalizer::new()
        .normalize(&make_record(&json))
        .unwrap();
    assert_eq!(
        format!("{:?}", result.evidence_status),
        "Pass",
        "the §24 `pass` verdict is `EvidenceStatus::Pass`"
    );
}

/// The normalizer maps `verdict: infrastructure_error`
/// to `NormalizationError::Unclassifiable(...)`. The
/// runtime's 1A.3 normalizer-error path turns this
/// into `ExecutorError::InfrastructureFailed` and
/// `GateStatus::Blocked` (§24).
#[test]
fn normalizer_maps_infrastructure_error_to_unclassifiable() {
    let report = DebianIssueReportV1 {
        schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
        report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
        verdict: IssueVerdict::InfrastructureError,
        source_package: "example".to_string(),
        snapshot_at: "2026-10-09T00:00:00Z".to_string(),
        bugs: Vec::new(),
        summary: BugSummary::default(),
    };
    let json = serde_json::to_string(&report).unwrap();
    let err = DebianBtsSnapshotNormalizer::new()
        .normalize(&make_record(&json))
        .unwrap_err();
    match err {
        ironmaint_executor::NormalizationError::Unclassifiable(msg) => {
            assert!(
                msg.contains("debian.issue.bts_snapshot"),
                "the 1E.2 unclassifiable message names the routing key: {msg}"
            );
        }
        other => panic!("expected Unclassifiable, got {other:?}"),
    }
}
