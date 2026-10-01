//! `FixtureNormalizer` — converts fixture subprocess records to
//! `NormalizedResult`. Lives here (not in `ironmaint-evidence`)
//! because it consumes executor-owned types.
//!
//! `Outcome` and `outcome_from_record` (§32, §92) classify an
//! `ExecutionRecord` into the §92 exit-checkpoint taxonomy. They
//! are kept adjacent to the normalizer because both interpret
//! exit codes the same way.

use ironmaint_evidence::EvidenceStatus;

use crate::normalizer::{NormalizationError, NormalizedResult, Observation, ResultNormalizer};
use crate::record::ExecutionRecord;

/// §92 exit-checkpoint classification of a tool run.
///
/// `Timeout` is produced only from the executor's error path
/// (`ExecutorErrorKind::ToolFailed { timed_out: true }`), never by
/// [`outcome_from_record`] — a record always carries an `exit_code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Pass,
    Fail,
    Timeout,
    Interrupted,
    InfrastructureFailed,
}

impl Outcome {
    /// Stable wire-format name (snake_case).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
            Self::InfrastructureFailed => "infrastructure_failed",
        }
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Map a successful `ExecutionRecord` to an [`Outcome`].
///
/// Exit-code mapping (PHASE-0B.md §32, §92):
/// - `0` → `Pass`
/// - `1` → `Fail` (tool's own failure exit)
/// - `127` → `InfrastructureFailed` (POSIX "command not found")
/// - `130` → `Interrupted` (POSIX SIGINT exit convention)
/// - other non-zero → `Fail` (unknown non-zero exit)
#[must_use]
pub fn outcome_from_record(record: &ExecutionRecord) -> Outcome {
    match record.exit_code {
        0 => Outcome::Pass,
        127 => Outcome::InfrastructureFailed,
        130 => Outcome::Interrupted,
        _ => Outcome::Fail,
    }
}

/// Exit-code-based normalizer with stdout-line observations.
pub struct FixtureNormalizer;

impl ResultNormalizer for FixtureNormalizer {
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
            retry_class: RetryClass::Safe,
            started_at: now,
            finished_at: now,
            exit_code,
            stdout: stdout.to_string(),
            stderr: String::new(),
            retries_exhausted: false,
            truncated: false,
            // The fixture tools are in-process; nothing spilled.
            artifacts: Vec::new(),
            artifacts_dropped: Vec::new(),
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

    #[test]
    fn outcome_pass_for_zero_exit() {
        assert_eq!(outcome_from_record(&rec(0, "")), Outcome::Pass);
    }

    #[test]
    fn outcome_fail_for_one_exit() {
        assert_eq!(outcome_from_record(&rec(1, "")), Outcome::Fail);
    }

    #[test]
    fn outcome_infrastructure_failed_for_127() {
        assert_eq!(
            outcome_from_record(&rec(127, "")),
            Outcome::InfrastructureFailed
        );
    }

    #[test]
    fn outcome_interrupted_for_130() {
        assert_eq!(outcome_from_record(&rec(130, "")), Outcome::Interrupted);
    }

    #[test]
    fn outcome_fail_for_other_nonzero() {
        assert_eq!(outcome_from_record(&rec(2, "")), Outcome::Fail);
        assert_eq!(outcome_from_record(&rec(-1, "")), Outcome::Fail);
    }

    #[test]
    fn outcome_wire_strings_are_snake_case() {
        assert_eq!(Outcome::Pass.as_str(), "pass");
        assert_eq!(Outcome::Fail.as_str(), "fail");
        assert_eq!(Outcome::Timeout.as_str(), "timeout");
        assert_eq!(Outcome::Interrupted.as_str(), "interrupted");
        assert_eq!(
            Outcome::InfrastructureFailed.as_str(),
            "infrastructure_failed"
        );
    }
}
