//! Smoke tests for ToolRegistry + ResultNormalizer trait.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_evidence::EvidenceStatus;
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRecord, NormalizationError, NormalizedResult,
    Observation, ResultNormalizer, RetryClass, ToolDefinitionRecord, ToolRegistry,
};
use time::OffsetDateTime;

#[derive(Clone)]
struct ExitCodeNormalizer;
impl ResultNormalizer for ExitCodeNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        let output_truncated = record.truncated;
        if record.exit_code == 0 {
            Ok(NormalizedResult {
                evidence_status: EvidenceStatus::Pass,
                output_truncated,
                observations: vec![],
                invalidations: vec![],
            })
        } else {
            Ok(NormalizedResult {
                evidence_status: EvidenceStatus::Fail,
                output_truncated,
                observations: record
                    .stdout
                    .lines()
                    .map(|l| Observation {
                        kind: "stdout-line".to_string(),
                        message: l.to_string(),
                    })
                    .collect(),
                invalidations: vec!["tool_exit_nonzero".to_string()],
            })
        }
    }
}

fn make_record(key: &str, normalizer: bool) -> ToolDefinitionRecord {
    let mut rec = ToolDefinitionRecord::new(
        ToolCapabilityKey::new(key).unwrap(),
        PathBuf::from("/usr/bin/ironmaint-fixture"),
        vec![OsString::from("--validate")],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    );
    if normalizer {
        rec = rec.with_normalizer(Arc::new(ExitCodeNormalizer));
    }
    rec
}

#[test]
fn registry_insert_and_get() {
    let mut r = ToolRegistry::new();
    r.register(Box::new(make_record("synthetic.build.with_norm", true)))
        .unwrap();
    r.register(Box::new(make_record("synthetic.qa.no_norm", false)))
        .unwrap();
    assert_eq!(r.len(), 2);
    let key = ToolCapabilityKey::new("synthetic.build.with_norm").unwrap();
    let tool = r.get(&key).expect("registered");
    assert!(tool.normalizer().is_some());
    assert_eq!(tool.key().as_str(), "synthetic.build.with_norm");
    assert_eq!(
        tool.executable(),
        PathBuf::from("/usr/bin/ironmaint-fixture").as_path()
    );
    assert_eq!(tool.fixed_args(), &[OsString::from("--validate")]);
    assert_eq!(tool.class(), ExecutionClass::Check);
}

#[test]
fn registry_rejects_duplicate() {
    let mut r = ToolRegistry::new();
    r.register(Box::new(make_record("synthetic.build.with_norm", true)))
        .unwrap();
    assert!(
        r.register(Box::new(make_record("synthetic.build.with_norm", true)))
            .is_err()
    );
}

fn rec(exit_code: i32, stdout: &str) -> ExecutionRecord {
    let now = OffsetDateTime::now_utc();
    ExecutionRecord {
        tool_key: ToolCapabilityKey::new("synthetic.build.with_norm").unwrap(),
        retry_class: RetryClass::Safe,
        started_at: now,
        finished_at: now,
        exit_code,
        stdout: stdout.to_string(),
        stderr: String::new(),
        retries_exhausted: false,
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
        truncated: false,
    }
}

#[test]
fn exit_code_normalizer_zero_is_pass() {
    let n = ExitCodeNormalizer;
    let r = n.normalize(&rec(0, "ok")).unwrap();
    assert_eq!(r.evidence_status, EvidenceStatus::Pass);
}

#[test]
fn exit_code_normalizer_nonzero_is_fail_with_lines() {
    let n = ExitCodeNormalizer;
    let r = n.normalize(&rec(1, "first\nsecond")).unwrap();
    assert_eq!(r.evidence_status, EvidenceStatus::Fail);
    assert_eq!(r.observations.len(), 2);
    assert_eq!(r.invalidations, vec!["tool_exit_nonzero".to_string()]);
}
