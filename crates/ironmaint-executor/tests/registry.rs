//! Smoke tests for ToolRegistry + ResultNormalizer trait.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_evidence::EvidenceStatus;
use ironmaint_executor::{
    ExecutionRecord, NormalizationError, NormalizedResult, Observation, ResultNormalizer,
    RetryClass, ToolDefinition, ToolRegistry,
};
use time::OffsetDateTime;

struct ExitCodeNormalizer;
impl ResultNormalizer for ExitCodeNormalizer {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError> {
        if record.exit_code == 0 {
            Ok(NormalizedResult {
                evidence_status: EvidenceStatus::Pass,
                observations: vec![],
                invalidations: vec![],
            })
        } else {
            Ok(NormalizedResult {
                evidence_status: EvidenceStatus::Fail,
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

struct WithNormalizer;
impl ToolDefinition for WithNormalizer {
    fn key(&self) -> &ToolCapabilityKey {
        static K: std::sync::OnceLock<ToolCapabilityKey> = std::sync::OnceLock::new();
        K.get_or_init(|| ToolCapabilityKey::new("synthetic.build.with_norm").unwrap())
    }
    fn normalizer(&self) -> Option<Box<dyn ResultNormalizer>> {
        Some(Box::new(ExitCodeNormalizer))
    }
}

struct NoNormalizer;
impl ToolDefinition for NoNormalizer {
    fn key(&self) -> &ToolCapabilityKey {
        static K: std::sync::OnceLock<ToolCapabilityKey> = std::sync::OnceLock::new();
        K.get_or_init(|| ToolCapabilityKey::new("synthetic.qa.no_norm").unwrap())
    }
    fn normalizer(&self) -> Option<Box<dyn ResultNormalizer>> {
        None
    }
}

#[test]
fn registry_insert_and_get() {
    let mut r = ToolRegistry::new();
    r.register(Box::new(WithNormalizer)).unwrap();
    r.register(Box::new(NoNormalizer)).unwrap();
    assert_eq!(r.len(), 2);
    let key = ToolCapabilityKey::new("synthetic.build.with_norm").unwrap();
    let tool = r.get(&key).expect("registered");
    assert!(tool.normalizer().is_some());
}

#[test]
fn registry_rejects_duplicate() {
    let mut r = ToolRegistry::new();
    r.register(Box::new(WithNormalizer)).unwrap();
    assert!(r.register(Box::new(WithNormalizer)).is_err());
}

fn rec(exit_code: i32, stdout: &str) -> ExecutionRecord {
    let now = OffsetDateTime::now_utc();
    ExecutionRecord {
        tool_key: ToolCapabilityKey::new("synthetic.build.with_norm").unwrap(),
        retry_class: RetryClass::Idempotent,
        started_at: now,
        finished_at: now,
        exit_code,
        stdout: stdout.to_string(),
        stderr: String::new(),
        retries_exhausted: false,
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
