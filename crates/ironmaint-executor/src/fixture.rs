//! `FixtureNormalizer` — converts fixture subprocess records to
//! `NormalizedResult`. Lives here (not in `ironmaint-evidence`)
//! because it consumes executor-owned types.

use ironmaint_evidence::EvidenceStatus;

use crate::normalizer::{NormalizationError, NormalizedResult, Observation, ResultNormalizer};
use crate::record::ExecutionRecord;

/// Exit-code-based normalizer with stdout-line observations.
pub struct FixtureNormalizer;

impl ResultNormalizer for FixtureNormalizer {
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
                        kind: "fixture-line".to_string(),
                        message: l.to_string(),
                    })
                    .collect(),
                invalidations: vec!["fixture_exit_nonzero".to_string()],
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::RetryClass;
    use ironmaint_adapter_api::ToolCapabilityKey;
    use time::OffsetDateTime;

    fn rec(exit_code: i32, stdout: &str) -> ExecutionRecord {
        let now = OffsetDateTime::now_utc();
        ExecutionRecord {
            tool_key: ToolCapabilityKey::new("synthetic.build.validate").unwrap(),
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
    fn zero_is_pass() {
        let n = FixtureNormalizer;
        let r = n.normalize(&rec(0, "ok")).unwrap();
        assert_eq!(r.evidence_status, EvidenceStatus::Pass);
        assert!(r.invalidations.is_empty());
    }

    #[test]
    fn nonzero_is_fail_with_lines() {
        let n = FixtureNormalizer;
        let r = n.normalize(&rec(1, "first\nsecond")).unwrap();
        assert_eq!(r.evidence_status, EvidenceStatus::Fail);
        assert_eq!(r.observations.len(), 2);
        assert_eq!(r.invalidations, vec!["fixture_exit_nonzero".to_string()]);
    }
}
