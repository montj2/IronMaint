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

use crate::report::{DebianSourcePreparationV1, DebianSourceReportV1, Verdict};

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
        let report: DebianSourceReportV1 = serde_json::from_str(record.stdout.as_str())
            .map_err(|e| {
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
