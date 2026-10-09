//! PR 1A.3 — `ResultNormalizer` is authoritative.
//!
//! The `ResultNormalizer` trait (PHASE-1.md §7) has been declared
//! since 0B.4 — every `ToolDefinitionRecord` carries an
//! `Option<Arc<dyn ResultNormalizer>>` — but the runtime has
//! never called it. Today, every record is classified by
//! `ironmaint_executor::outcome_from_record`, which maps
//! `exit_code == 0` to `Pass` and any other value to `Fail`.
//!
//! PR 1A.3 makes the normalizer authoritative: a tool that
//! declares one gets its `ExecutionRecord` classified by
//! `n.normalize(&record)`, and the result carries
//! `evidence_status`, an `output_truncated` flag, and a list
//! of `observations`. A tool that does not declare one keeps
//! the exit-code fallback. The runtime remains the only writer
//! of `EvidenceStatus`; the normalizer's `evidence_status` is
//! the input, not the decision.
//!
//! The teeth-check is `structured_report_normalizer_makes_pass`:
//! it defines a `StructuredReportNormalizer` that parses the
//! child's stdout as JSON, returns `Pass` when the JSON says
//! `{"status": "pass"}`, and `Fail` otherwise. Without the
//! runtime's normalizer invocation the test would never reach
//! the normalizer, so a green run of this file is the proof
//! that 1A.3 landed.
//!
//! `unwrap`/`expect` are allowed in this test module because
//! failure should panic; the production crate forbids them via
//! the workspace lint table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use ironmaint_evidence::EvidenceStatus;
use ironmaint_executor::record::ExecutionRecord;
use ironmaint_executor::{
    NormalizationError, NormalizedResult, Observation, ResultNormalizer, outcome_from_record,
};
use time::OffsetDateTime;

/// A normalizer that classifies a record by parsing the child's
/// stdout as `{"status": "pass"|"fail", "observations": [...]}`.
///
/// The shape matches the `synthetic.build.structured_report`
/// fixture's output (PR 1A.3 GREEN) and, more importantly, the
/// shape every Phase 1 inspection tool (1C.x) is expected to
/// emit: a small, documented JSON object whose `status` is
/// the only field the runtime reads, with `observations` as
/// free-form human notes the runtime does not interpret.
///
/// A malformed body returns `NormalizationError::Malformed`
/// rather than a best-effort Fail — the runtime maps that to
/// a tool-level Fail, not an infrastructure error, because a
/// tool that emits garbage is not a tool the runtime
/// miscounted.
#[derive(Debug)]
struct StructuredReportNormalizer;

impl ResultNormalizer for StructuredReportNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        let v: serde_json::Value = serde_json::from_str(&record.stdout)
            .map_err(|e| NormalizationError::Malformed(format!("not valid JSON: {e}")))?;
        let status = v
            .get("status")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                NormalizationError::Malformed("missing string `status` field".to_string())
            })?;
        let observations = v
            .get("observations")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        let kind = o.get("kind").and_then(serde_json::Value::as_str)?;
                        let message = o.get("message").and_then(serde_json::Value::as_str)?;
                        Some(Observation {
                            kind: kind.to_string(),
                            message: message.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let evidence_status = match status {
            "pass" => EvidenceStatus::Pass,
            "fail" => EvidenceStatus::Fail,
            other => {
                return Err(NormalizationError::Unclassifiable(format!(
                    "unknown status `{other}`"
                )));
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

fn make_record(stdout: &str, exit_code: i32, truncated: bool) -> ExecutionRecord {
    ExecutionRecord {
        tool_key: ironmaint_adapter_api::ToolCapabilityKey::new(
            "synthetic.build.structured_report",
        )
        .unwrap(),
        retry_class: ironmaint_executor::RetryClass::Safe,
        started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        finished_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        exit_code,
        stdout: stdout.to_string(),
        stderr: String::new(),
        retries_exhausted: false,
        truncated,
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    }
}

#[test]
fn structured_report_normalizer_makes_pass() {
    let n = StructuredReportNormalizer;
    let rec = make_record(r#"{"status":"pass","observations":[]}"#, 1, false);
    // Note: `exit_code == 1`. The exit-code fallback would
    // return Fail. The normalizer says Pass. 1A.3 is the
    // branch that makes the normalizer authoritative.
    let result = n.normalize(&rec).expect("normalize");
    assert_eq!(result.evidence_status, EvidenceStatus::Pass);
    assert!(!result.output_truncated);
}

#[test]
fn structured_report_normalizer_makes_fail() {
    let n = StructuredReportNormalizer;
    let rec = make_record(
        r#"{"status":"fail","observations":[{"kind":"lint","message":"trailing-whitespace"}]}"#,
        0,
        false,
    );
    // exit_code == 0; the exit-code fallback would return
    // Pass. The structured report says Fail with a
    // single observation.
    let result = n.normalize(&rec).expect("normalize");
    assert_eq!(result.evidence_status, EvidenceStatus::Fail);
    assert_eq!(result.observations.len(), 1);
    assert_eq!(result.observations[0].kind, "lint");
    assert_eq!(result.observations[0].message, "trailing-whitespace");
}

#[test]
fn structured_report_normalizer_propagates_truncation() {
    let n = StructuredReportNormalizer;
    let rec = make_record(r#"{"status":"pass"}"#, 0, true);
    let result = n.normalize(&rec).expect("normalize");
    assert!(result.output_truncated);
}

#[test]
fn structured_report_normalizer_rejects_malformed_body() {
    let n = StructuredReportNormalizer;
    let rec = make_record("not json at all", 0, false);
    let err = n
        .normalize(&rec)
        .expect_err("malformed body must be a NormalizationError");
    assert!(
        matches!(err, NormalizationError::Malformed(_)),
        "malformed body must map to Malformed, got {err:?}"
    );
}

#[test]
fn structured_report_normalizer_rejects_unknown_status() {
    let n = StructuredReportNormalizer;
    let rec = make_record(r#"{"status":"maybe"}"#, 0, false);
    let err = n
        .normalize(&rec)
        .expect_err("unknown status must be a NormalizationError");
    assert!(
        matches!(err, NormalizationError::Unclassifiable(_)),
        "unknown status must map to Unclassifiable, got {err:?}"
    );
}

/// `outcome_from_record` is the *fallback* for tools that do
/// not declare a normalizer — pin the contract so 1A.3 does
/// not accidentally widen it.
#[test]
fn outcome_from_record_exit_zero_is_pass_and_nonzero_is_fail() {
    let ok = make_record("anything", 0, false);
    let bad = make_record("anything", 1, false);
    assert_eq!(outcome_from_record(&ok), ironmaint_executor::Outcome::Pass);
    assert_eq!(outcome_from_record(&bad), ironmaint_executor::Outcome::Fail);
}

/// Constructor: the runtime needs a way to look up the
/// normalizer from a `ToolDefinition` and pass it an
/// `ExecutionRecord`. This test pins the round-trip through
/// `ToolDefinitionRecord` so a future refactor of the trait
/// method that drops the accessor surfaces here.
#[test]
fn tool_definition_record_carries_the_normalizer() {
    use ironmaint_executor::ToolDefinition;
    let ck =
        ironmaint_adapter_api::ToolCapabilityKey::new("synthetic.build.structured_report").unwrap();
    let record = ironmaint_executor::ToolDefinitionRecord::new(
        ck,
        PathBuf::from("/usr/bin/ironmaint-fixture"),
        vec![],
        ironmaint_executor::ExecutionClass::Check,
        ironmaint_executor::ExecutionLimits::default(),
    )
    .with_normalizer(Arc::new(StructuredReportNormalizer) as Arc<dyn ResultNormalizer>);
    let normalizer = record.normalizer().expect("normalizer must be Some");
    let rec = make_record(r#"{"status":"pass"}"#, 1, false);
    let result = normalizer.normalize(&rec).expect("normalize");
    assert_eq!(result.evidence_status, EvidenceStatus::Pass);
}
