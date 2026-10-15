//! `DebianSourcePreparationNormalizer` — the
//! `ironmaint_executor::ResultNormalizer` that classifies the
//! 1C.1 tool's stdout.
//!
//! PHASE-1.md §17: the tool's `verdict` is the
//! `pass | fail | infrastructure_error` tri-state. The
//! normalizer maps each to a `NormalizedResult`:
//!
//!   verdict: "pass"               → EvidenceStatus::Pass
//!   verdict: "fail"               → EvidenceStatus::Fail
//!   verdict: "infrastructure_error"
//!     → Err(NormalizationError::Unclassifiable)
//!
//! The `Err` arm is what the runtime (1A.3) reads as
//! `ExecutorError::InfrastructureFailed` — distinct from a
//! tool exit-1 (which the executor's exit-code classifier
//! would treat as `EvidenceStatus::Fail`). The
//! distinction is the load-bearing property of §17: a
//! *package* problem (`Fail`) is not a *toolchain* problem
//! (`InfrastructureError`).
//!
//! The normalizer also propagates the report's `diagnostics`
//! as `Observation`s, so an IronClaw repair loop can read
//! the specific failure reasons ("control's Source:
//! `example` does not match candidate source name
//! `foo`") without having to re-parse the report.

use ironmaint_evidence::EvidenceStatus;
use ironmaint_executor::{
    ExecutionRecord, NormalizationError, NormalizedResult, Observation, ResultNormalizer,
};

use crate::report::{
    DebianIssueReportV1, DebianMaintenanceContextV1, DebianPolicyDeltaReviewV1,
    DebianSourcePreparationV1, DebianSourceReportV1, IssueVerdict, Verdict,
};

/// The 1C.1 normalizer. Stateless; cheap to construct.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebianSourcePreparationNormalizer;

impl DebianSourcePreparationNormalizer {
    /// Constructor used by the daemon's tool registry:
    /// `Arc::new(DebianSourcePreparationNormalizer::new())`.
    /// The body is `Self::default()`; the constructor
    /// exists for call-site readability and so future
    /// state can be added without changing every call
    /// site.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ResultNormalizer for DebianSourcePreparationNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        let report: DebianSourcePreparationV1 = serde_json::from_str(record.stdout.as_str())
            .map_err(|e| {
                NormalizationError::Malformed(format!(
                    "could not parse DebianSourcePreparationV1 from tool stdout: {e}"
                ))
            })?;

        let observations = report
            .diagnostics
            .iter()
            .map(|d| Observation {
                kind: d.code.clone(),
                message: d.message.clone(),
            })
            .collect();

        let evidence_status = match report.verdict {
            Verdict::Pass => EvidenceStatus::Pass,
            Verdict::Fail => EvidenceStatus::Fail,
            Verdict::InfrastructureError => {
                return Err(NormalizationError::Unclassifiable(
                    "debian.inspect.source_preparation: infrastructure error".to_string(),
                ));
            }
        };

        Ok(NormalizedResult {
            evidence_status,
            output_truncated: record.truncated,
            observations,
            invalidations: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{
        ChangelogIdentity, DebianSourcePreparationV1, Diagnostic, REPORT_KIND, SCHEMA_VERSION,
        SourcePreparationFindings, Verdict,
    };
    use ironmaint_adapter_api::ToolCapabilityKey;
    use ironmaint_executor::RetryClass;
    use time::macros::datetime;

    /// Build a minimal `ExecutionRecord` for normalizer
    /// tests. The normalizer reads only `stdout` (to
    /// parse the report) and `truncated` (to propagate
    /// into `NormalizedResult.output_truncated`); the
    /// other fields are pinned to representative
    /// defaults so the test fixtures are stable.
    fn make_record(stdout: &str) -> ExecutionRecord {
        ExecutionRecord {
            tool_key: ToolCapabilityKey::new("debian.inspect.source_preparation").unwrap(),
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

    /// A `verdict: "pass"` report normalises to
    /// `EvidenceStatus::Pass` with no observations.
    #[test]
    fn pass_verdict_normalises_to_pass() {
        let report = DebianSourcePreparationV1 {
            schema_version: SCHEMA_VERSION,
            report_kind: REPORT_KIND.to_string(),
            candidate_fingerprint: "blake3:pass".to_string(),
            verdict: Verdict::Pass,
            diagnostics: vec![],
            findings: SourcePreparationFindings {
                debian_directory_present: true,
                control_parsable: true,
                control_source_name: Some("example".to_string()),
                control_source_name_matches_candidate: true,
                changelog_parsable: true,
                changelog_source_name: Some("example".to_string()),
                changelog_source_name_matches_candidate: true,
                changelog_version: Some("1.0.0-1".to_string()),
                changelog_version_matches_candidate: true,
                format_identifiable: true,
                source_format: Some("3.0 (quilt)".to_string()),
                rules_executable: true,
            },
            identity: Some(ChangelogIdentity {
                source: "example".to_string(),
                version: "1.0.0-1".to_string(),
                distribution: "unstable".to_string(),
                urgency: "medium".to_string(),
                ..ChangelogIdentity::default()
            }),
        };
        let json = serde_json::to_string(&report).unwrap();
        let result = DebianSourcePreparationNormalizer::new()
            .normalize(&make_record(&json))
            .unwrap();
        assert_eq!(result.evidence_status, EvidenceStatus::Pass);
        assert!(result.observations.is_empty());
        assert!(result.invalidations.is_empty());
        assert!(!result.output_truncated);
    }

    /// A `verdict: "fail"` report normalises to
    /// `EvidenceStatus::Fail` with the diagnostics as
    /// observations.
    #[test]
    fn fail_verdict_normalises_to_fail_with_observations() {
        let report = DebianSourcePreparationV1 {
            schema_version: SCHEMA_VERSION,
            report_kind: REPORT_KIND.to_string(),
            candidate_fingerprint: "blake3:fail".to_string(),
            verdict: Verdict::Fail,
            diagnostics: vec![Diagnostic {
                code: "E_VERSION_MISMATCH".to_string(),
                message: "debian/changelog version 1.0.0-1 does not match candidate version 2.0.0"
                    .to_string(),
                path: Some(std::path::PathBuf::from("debian/changelog")),
            }],
            findings: SourcePreparationFindings {
                debian_directory_present: true,
                control_parsable: true,
                control_source_name: Some("example".to_string()),
                control_source_name_matches_candidate: true,
                changelog_parsable: true,
                changelog_source_name: Some("example".to_string()),
                changelog_source_name_matches_candidate: true,
                changelog_version: Some("1.0.0-1".to_string()),
                changelog_version_matches_candidate: false,
                format_identifiable: true,
                source_format: Some("3.0 (quilt)".to_string()),
                rules_executable: true,
            },
            identity: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        let result = DebianSourcePreparationNormalizer::new()
            .normalize(&make_record(&json))
            .unwrap();
        assert_eq!(result.evidence_status, EvidenceStatus::Fail);
        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].kind, "E_VERSION_MISMATCH");
        assert!(
            result.observations[0]
                .message
                .contains("does not match candidate version")
        );
        assert!(result.invalidations.is_empty());
    }

    /// A `verdict: "infrastructure_error"` report
    /// normalises to `Err(NormalizationError::Unclassifiable(...))`.
    /// The runtime's 1A.3 normalizer-error path turns
    /// this into `ExecutorError::InfrastructureFailed`.
    #[test]
    fn infrastructure_error_verdict_normalises_to_unclassifiable() {
        let report = DebianSourcePreparationV1 {
            schema_version: SCHEMA_VERSION,
            report_kind: REPORT_KIND.to_string(),
            candidate_fingerprint: "blake3:infra".to_string(),
            verdict: Verdict::InfrastructureError,
            diagnostics: vec![Diagnostic {
                code: "E_WORKSPACE_NOT_DIR".to_string(),
                message: "workspace_path /var/lib/ironmaint/workspaces/abc is not a directory"
                    .to_string(),
                path: None,
            }],
            findings: SourcePreparationFindings {
                debian_directory_present: false,
                control_parsable: false,
                control_source_name: None,
                control_source_name_matches_candidate: false,
                changelog_parsable: false,
                changelog_source_name: None,
                changelog_source_name_matches_candidate: false,
                changelog_version: None,
                changelog_version_matches_candidate: false,
                format_identifiable: false,
                source_format: None,
                rules_executable: false,
            },
            identity: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        let err = DebianSourcePreparationNormalizer::new()
            .normalize(&make_record(&json))
            .unwrap_err();
        assert!(matches!(err, NormalizationError::Unclassifiable(_)));
    }

    /// A stdout that is not valid JSON is a `Malformed`
    /// error (the runtime turns this into the same
    /// `ExecutorError::InfrastructureFailed` arm —
    /// "tool ran but produced garbage" is a toolchain
    /// problem, not a package problem).
    #[test]
    fn malformed_stdout_normalises_to_malformed_error() {
        let err = DebianSourcePreparationNormalizer::new()
            .normalize(&make_record("not valid json"))
            .unwrap_err();
        assert!(matches!(err, NormalizationError::Malformed(_)));
    }
}

// =============================================================================
// 1C.2 — `DebianSourceAnalysisNormalizer` (PHASE-1.md §17, §18).
//
// Mirrors the 1C.1 normalizer; the verdict tri-state mapping
// is identical. The unclassifiable message is the
// 1C.2-specific string so a runtime log / observation can
// distinguish the two normalizers.
// =============================================================================

/// The 1C.2 normalizer. Stateless; cheap to construct.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebianSourceAnalysisNormalizer;

impl DebianSourceAnalysisNormalizer {
    /// Constructor used by the daemon's tool registry:
    /// `Arc::new(DebianSourceAnalysisNormalizer::new())`.
    /// The body is `Self::default()`; the constructor
    /// exists for call-site readability.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ResultNormalizer for DebianSourceAnalysisNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        let report: DebianSourceReportV1 =
            serde_json::from_str(record.stdout.as_str()).map_err(|e| {
                NormalizationError::Malformed(format!(
                    "could not parse DebianSourceReportV1 from tool stdout: {e}"
                ))
            })?;

        let observations = report
            .diagnostics
            .iter()
            .map(|d| Observation {
                kind: d.code.clone(),
                message: d.message.clone(),
            })
            .collect();

        let evidence_status = match report.verdict {
            Verdict::Pass => EvidenceStatus::Pass,
            Verdict::Fail => EvidenceStatus::Fail,
            Verdict::InfrastructureError => {
                return Err(NormalizationError::Unclassifiable(
                    "debian.inspect.source_analysis: infrastructure error".to_string(),
                ));
            }
        };

        Ok(NormalizedResult {
            evidence_status,
            output_truncated: record.truncated,
            observations,
            invalidations: Vec::new(),
        })
    }
}

// =============================================================================
// 1C.3 — `DebianMaintenanceContextNormalizer` (PHASE-1.md §17, §19, §26).
//
// Mirrors the 1C.1 / 1C.2 normalizers; the verdict tri-state
// mapping is identical. The unclassifiable message is the
// 1C.3-specific string so a runtime log / observation can
// distinguish the three normalizers.
// =============================================================================

/// The 1C.3 normalizer. Stateless; cheap to construct.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebianMaintenanceContextNormalizer;

impl DebianMaintenanceContextNormalizer {
    /// Constructor used by the daemon's tool registry:
    /// `Arc::new(DebianMaintenanceContextNormalizer::new())`.
    /// The body is `Self::default()`; the constructor
    /// exists for call-site readability.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ResultNormalizer for DebianMaintenanceContextNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        let report: DebianMaintenanceContextV1 = serde_json::from_str(record.stdout.as_str())
            .map_err(|e| {
                NormalizationError::Malformed(format!(
                    "could not parse DebianMaintenanceContextV1 from tool stdout: {e}"
                ))
            })?;

        let observations = report
            .diagnostics
            .iter()
            .map(|d| Observation {
                kind: d.code.clone(),
                message: d.message.clone(),
            })
            .collect();

        let evidence_status = match report.verdict {
            Verdict::Pass => EvidenceStatus::Pass,
            Verdict::Fail => EvidenceStatus::Fail,
            Verdict::InfrastructureError => {
                return Err(NormalizationError::Unclassifiable(
                    "debian.inspect.maintenance_context: infrastructure error".to_string(),
                ));
            }
        };

        Ok(NormalizedResult {
            evidence_status,
            output_truncated: record.truncated,
            observations,
            invalidations: Vec::new(),
        })
    }
}

// =============================================================================
// 1D.2 — `DebianPolicyDeltaReviewNormalizer` (PHASE-1.md §13, §19, §22, §17).
//
// Maps the 1D.2 report's tri-state verdict to the §17
// tri-state `EvidenceStatus`:
// `verdict: "pass"` → `EvidenceStatus::Pass`,
// `verdict: "fail"` → `EvidenceStatus::Fail` (with the
// `E_STANDARDS_VERSION_MISSING` / `E_POLICY_BASELINE_STALE`
// diagnostic propagated as an observation), and
// `verdict: "infrastructure_error"` →
// `EvidenceStatus::InfrastructureError`.
//
// PHASE-1.1 PR 1.1.D (policy baseline correctness):
// the pre-1.1.D normalizer mapped
// `verdict: "infrastructure_error"` to
// `NormalizationError::Unclassifiable`, which the
// runtime's `Err(e)` arm collapsed to
// `Outcome::Fail` (the runtime's comment notes that
// "the tool ran to completion and the runtime received
// its output" → not an infrastructure problem). That
// mapping was correct for the pre-1.1.D code path (the
// only way to reach `Verdict::InfrastructureError` was
// a workspace I/O error, which is exactly the case the
// runtime's `Unclassifiable → Fail` mapping is for).
//
// Post-1.1.D, the §22 gate (`AboveBaseline` case)
// emits `Verdict::InfrastructureError` for a
// "manifest is stale, refresh the authority snapshot"
// case — which is *not* a tool failure, it is an
// infrastructure problem the operator must resolve.
// The §17 tri-state is `Pass / Fail /
// InfrastructureError`, and the spec is explicit
// (line 978) that an above-baseline candidate "must
// not be marked bad". The normalizer therefore
// surfaces the verdict directly to
// `EvidenceStatus::InfrastructureError`, which the
// runtime maps to `Outcome::InfrastructureFailed`
// → `GateStatus::Blocked`. The pre-1.1.D
// "workspace I/O error" case still maps to
// `Verdict::InfrastructureError`, but the runtime's
// `Outcome::Fail` mapping (driven by the
// `Fail`-arm fallback for unclassifiable errors)
// is no longer the right destination for either
// case: the §17 tri-state requires the
// `InfrastructureError` evidence status to reach
// the runtime's `GateStatus::Blocked` path. The
// runtime's `Outcome` arm maps
// `InfrastructureFailed` → `InfrastructureError`
// evidence; the `Outcome::Fail` arm maps to
// `Fail` evidence.
// =============================================================================

/// The 1D.2 `ResultNormalizer` implementation.
pub struct DebianPolicyDeltaReviewNormalizer;

impl DebianPolicyDeltaReviewNormalizer {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianPolicyDeltaReviewNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultNormalizer for DebianPolicyDeltaReviewNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        let report: DebianPolicyDeltaReviewV1 = serde_json::from_str(record.stdout.as_str())
            .map_err(|e| {
                NormalizationError::Malformed(format!(
                    "could not parse DebianPolicyDeltaReviewV1 from tool stdout: {e}"
                ))
            })?;

        let observations = report
            .diagnostics
            .iter()
            .map(|d| Observation {
                kind: d.code.clone(),
                message: d.message.clone(),
            })
            .collect();

        // PHASE-1.1 PR 1.1.D: the `InfrastructureError`
        // verdict is now surfaced directly to the §17
        // tri-state's `EvidenceStatus::InfrastructureError`,
        // which the runtime maps to
        // `Outcome::InfrastructureFailed` →
        // `GateStatus::Blocked`. The pre-1.1.D code
        // returned `NormalizationError::Unclassifiable`,
        // which the runtime collapsed to `Outcome::Fail`
        // — losing the "manifest is stale, not the
        // package" signal the spec at line 978 requires.
        let evidence_status = match report.verdict {
            Verdict::Pass => EvidenceStatus::Pass,
            Verdict::Fail => EvidenceStatus::Fail,
            Verdict::InfrastructureError => EvidenceStatus::InfrastructureError,
        };

        Ok(NormalizedResult {
            evidence_status,
            output_truncated: record.truncated,
            observations,
            invalidations: Vec::new(),
        })
    }
}

// =============================================================================
// 1E.2 — `DebianBtsSnapshotNormalizer` (PHASE-1.md §23, §24, §17).
//
// The 1E.2 verdict is binary — `pass` or
// `infrastructure_error` — per §24. A populated bug
// list is *not* a failure (§24 "A package having 50
// open bugs is not a tool failure"). The
// `infrastructure_error` verdict maps to
// `NormalizationError::Unclassifiable(...)`, the same
// way the §17 `infrastructure_error` does in the
// 1C.x / 1D.x normalizers.
//
// The 1E.2 broken-then-fixed sentinel lives in the
// malformed-JSON branch. The RED commit maps
// malformed JSON to `NormalizationError::Malformed`,
// the same shape the 1C.x / 1D.x normalizers use.
// The GREEN commit maps malformed JSON to
// `NormalizationError::Unclassifiable` per §24
// "malformed service response" →
// `InfrastructureError → Blocked`. The runtime's
// 1A.3 normalizer-error path turns both variants into
// `ExecutorError::InfrastructureFailed`, so the
// runtime-level behaviour is the same; the difference
// is the type-level distinction a §24-aware reader
// can use to identify "the BTS service returned
// garbage" specifically (vs. "the tool returned
// garbage", which is the broader 1C.x / 1D.x case).
// =============================================================================

/// The 1E.2 `ResultNormalizer` implementation.
pub struct DebianBtsSnapshotNormalizer;

impl DebianBtsSnapshotNormalizer {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianBtsSnapshotNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultNormalizer for DebianBtsSnapshotNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        let report: DebianIssueReportV1 =
            serde_json::from_str(record.stdout.as_str()).map_err(|e| {
                // 1E.2 GREEN: per §24 "malformed service
                // response" → `InfrastructureError →
                // Blocked`, malformed JSON from the BTS
                // tool is `Unclassifiable` (the runtime's
                // 1A.3 path turns this into
                // `ExecutorError::InfrastructureFailed`).
                // The RED commit used `Malformed`, the
                // existing pattern from 1C.x / 1D.x.
                NormalizationError::Unclassifiable(format!(
                    "debian.issue.bts_snapshot: malformed service response: {e}"
                ))
            })?;

        let observations = Vec::new();

        let evidence_status = match report.verdict {
            IssueVerdict::Pass => EvidenceStatus::Pass,
            IssueVerdict::InfrastructureError => {
                return Err(NormalizationError::Unclassifiable(
                    "debian.issue.bts_snapshot: infrastructure error".to_string(),
                ));
            }
        };

        Ok(NormalizedResult {
            evidence_status,
            output_truncated: record.truncated,
            observations,
            invalidations: Vec::new(),
        })
    }
}

#[cfg(test)]
mod bts_snapshot_tests {
    use super::*;
    use crate::report::{
        BTS_SNAPSHOT_REPORT_KIND, BTS_SNAPSHOT_SCHEMA_VERSION, BugSummary, DebianIssueReportV1,
        IssueVerdict,
    };
    use ironmaint_adapter_api::ToolCapabilityKey;
    use ironmaint_executor::RetryClass;
    use time::macros::datetime;

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

    /// Build a minimal `DebianIssueReportV1` with
    /// the comprehensive fields populated. The
    /// `verdict` is `pass` and the bug list is
    /// empty (the normalizer never inspects the
    /// bug list per §24).
    fn pass_report() -> DebianIssueReportV1 {
        DebianIssueReportV1 {
            schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
            report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
            verdict: IssueVerdict::Pass,
            source_package: "example".to_string(),
            snapshot_at: "2026-10-09T00:00:00Z".to_string(),
            bugs: Vec::new(),
            summary: BugSummary::default(),
        }
    }

    /// A `verdict: "pass"` report normalises to
    /// `EvidenceStatus::Pass`. §24 "A package having
    /// 50 open bugs is not a tool failure" — the
    /// normalizer never inspects the bug list.
    #[test]
    fn pass_verdict_normalises_to_pass() {
        let report = pass_report();
        let json = serde_json::to_string(&report).unwrap();
        let result = DebianBtsSnapshotNormalizer::new()
            .normalize(&make_record(&json))
            .unwrap();
        assert_eq!(result.evidence_status, EvidenceStatus::Pass);
        assert!(result.observations.is_empty());
    }

    /// A `verdict: "infrastructure_error"` report
    /// normalises to `Err(NormalizationError::Unclassifiable(...))`.
    /// The runtime's 1A.3 normalizer-error path
    /// turns this into `ExecutorError::InfrastructureFailed`
    /// and `GateStatus::Blocked` (§24).
    #[test]
    fn infrastructure_error_verdict_normalises_to_unclassifiable() {
        let report = DebianIssueReportV1 {
            verdict: IssueVerdict::InfrastructureError,
            ..pass_report()
        };
        let json = serde_json::to_string(&report).unwrap();
        let err = DebianBtsSnapshotNormalizer::new()
            .normalize(&make_record(&json))
            .unwrap_err();
        assert!(matches!(err, NormalizationError::Unclassifiable(_)));
    }

    /// A stdout that is not valid JSON is a
    /// `Unclassifiable` error per §24 "malformed
    /// service response" → `InfrastructureError →
    /// Blocked`. The RED commit used `Malformed`
    /// (the 1C.x / 1D.x pattern); the GREEN commit
    /// uses `Unclassifiable` to signal "the BTS
    /// service returned garbage" specifically.
    #[test]
    fn malformed_stdout_normalises_to_unclassifiable() {
        let err = DebianBtsSnapshotNormalizer::new()
            .normalize(&make_record("not valid json"))
            .unwrap_err();
        assert!(
            matches!(err, NormalizationError::Unclassifiable(_)),
            "malformed JSON is `Unclassifiable` per §24, got {err:?}"
        );
    }
}
