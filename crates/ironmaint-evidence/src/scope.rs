//! Evidence scope.
//!
//! "Where does this evidence apply?" — the candidate, the job, or the
//! whole distribution. Most evidence is candidate-bound (§30 freshness);
//! a few kinds apply more broadly (e.g. distribution-wide policy
//! metadata).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ironmaint_core::{CandidateFingerprint, DistributionRef, JobId};

/// Where an [`Evidence`] object's findings apply.
///
/// `Candidate` is the default for build/test/reproducibility evidence
/// (bound to one source revision). `Job` covers evidence that
/// accumulates across multiple candidates (rare; e.g. aggregate
/// lint findings). `Distribution` covers evidence about the
/// distribution itself (e.g. archive metadata).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum EvidenceScope {
    Candidate(CandidateFingerprint),
    Job(JobId),
    Distribution(DistributionRef),
}

impl std::fmt::Display for EvidenceScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Candidate(fp) => write!(f, "candidate:{fp}"),
            Self::Job(id) => write!(f, "job:{id}"),
            Self::Distribution(d) => write!(f, "distribution:{}/{}", d.family, d.release),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{DistributionFamily, DistributionRelease};

    #[test]
    fn candidate_scope_round_trips() {
        let fp = CandidateFingerprint::from_hex("a".repeat(64)).unwrap();
        let scope = EvidenceScope::Candidate(fp);
        let json = serde_json::to_string(&scope).unwrap();
        let parsed: EvidenceScope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, scope);
    }

    #[test]
    fn job_scope_round_trips() {
        let scope = EvidenceScope::Job(JobId::new());
        let json = serde_json::to_string(&scope).unwrap();
        let parsed: EvidenceScope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, scope);
    }

    #[test]
    fn distribution_scope_round_trips() {
        let dr = DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        );
        let scope = EvidenceScope::Distribution(dr);
        let json = serde_json::to_string(&scope).unwrap();
        let parsed: EvidenceScope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, scope);
    }

    #[test]
    fn scope_display() {
        let fp = CandidateFingerprint::from_hex("0".repeat(64)).unwrap();
        assert!(
            EvidenceScope::Candidate(fp)
                .to_string()
                .starts_with("candidate:")
        );
        let dr = DistributionRef::new(
            DistributionFamily::new("fedora").unwrap(),
            DistributionRelease::new("rawhide").unwrap(),
        );
        assert_eq!(
            EvidenceScope::Distribution(dr).to_string(),
            "distribution:fedora/rawhide",
        );
    }
}
