//! Approval requirements and decisions (§40).
//!
//! Approval lives *outside* the state machine — the engine reads
//! [`ApprovalDecision`]s from the supplied context. An approval is
//! a recorded "yes" (or "no") from the appropriate actor; agents
//! propose [`ApprovalRequirement`]s, but only the privileged service
//! may record an [`ApprovalDecision`].

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ironmaint_core::{ApprovalId, CandidateFingerprint};

/// Category of approval required (§40).
///
/// `Other(String)` is the explicit escape hatch for distribution- or
/// org-specific categories the core vocabulary doesn't enumerate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalCategory {
    HumanReview,
    PolicyException,
    ExternalMutation,
    Publication,
    Signing,
    SecuritySensitive,
    /// Adapter- or org-specific categories.
    Other(String),
}

impl ApprovalCategory {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::HumanReview => "human_review",
            Self::PolicyException => "policy_exception",
            Self::ExternalMutation => "external_mutation",
            Self::Publication => "publication",
            Self::Signing => "signing",
            Self::SecuritySensitive => "security_sensitive",
            Self::Other(_) => "other",
        }
    }
}

impl fmt::Display for ApprovalCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(s) => write!(f, "other({s})"),
            other => f.write_str(other.name()),
        }
    }
}

/// Recorded decision against an [`ApprovalRequirement`] (§40).
///
/// Agents and adapters cannot record a decision; the privileged
/// service does. A `Rejected` decision means the corresponding
/// transition is blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

impl ApprovalDecision {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

impl fmt::Display for ApprovalDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

const DESCRIPTION_MAX: usize = 2048;

/// A recorded approval requirement (§40).
///
/// Created when an agent or adapter asserts "this transition needs
/// an approval of category X". The privileged service eventually
/// attaches an [`ApprovalDecision`] keyed by the requirement's id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRequirement {
    pub id: ApprovalId,
    pub candidate: CandidateFingerprint,
    pub category: ApprovalCategory,
    /// Human-readable description. Capped at 2 KiB.
    pub description: String,
}

impl ApprovalRequirement {
    /// Construct an [`ApprovalRequirement`].
    ///
    /// # Errors
    /// Returns `Err(&'static str)` if `description` exceeds
    /// [`DESCRIPTION_MAX`] bytes.
    pub fn new(
        candidate: CandidateFingerprint,
        category: ApprovalCategory,
        description: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let d = description.into();
        if d.len() > DESCRIPTION_MAX {
            return Err("approval description exceeds 2048 bytes");
        }
        Ok(Self {
            id: ApprovalId::new(),
            candidate,
            category,
            description: d,
        })
    }
}

/// A [`BTreeMap`]-backed registry of approval decisions keyed by
/// the [`ApprovalId`] they answer.
///
/// The state machine reads this to evaluate `MissingApproval` /
/// `Approved` predicates.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ApprovalRegistry {
    decisions: BTreeMap<ApprovalId, ApprovalDecision>,
}

impl ApprovalRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, id: ApprovalId, decision: ApprovalDecision) {
        self.decisions.insert(id, decision);
    }

    #[must_use]
    pub fn get(&self, id: ApprovalId) -> Option<ApprovalDecision> {
        self.decisions.get(&id).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (ApprovalId, ApprovalDecision)> {
        self.decisions.iter().map(|(k, v)| (*k, *v))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.decisions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::CandidateFingerprint;

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    #[test]
    fn category_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ApprovalCategory::SecuritySensitive).unwrap(),
            r#""security_sensitive""#
        );
    }

    #[test]
    fn category_other_carries_string() {
        let c = ApprovalCategory::Other("debian-ftp-master".to_string());
        let json = serde_json::to_string(&c).unwrap();
        let parsed: ApprovalCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn category_display() {
        assert_eq!(ApprovalCategory::HumanReview.to_string(), "human_review");
        assert_eq!(
            ApprovalCategory::Other("foo".into()).to_string(),
            "other(foo)"
        );
    }

    #[test]
    fn decision_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Approved).unwrap(),
            r#""approved""#
        );
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Rejected).unwrap(),
            r#""rejected""#
        );
    }

    #[test]
    fn approval_requirement_minimum() {
        let r = ApprovalRequirement::new(
            fp(),
            ApprovalCategory::HumanReview,
            "needs a maintainer review",
        )
        .unwrap();
        assert_eq!(r.description, "needs a maintainer review");
    }

    #[test]
    fn approval_requirement_rejects_oversize_description() {
        let long = "x".repeat(DESCRIPTION_MAX + 1);
        let err = ApprovalRequirement::new(fp(), ApprovalCategory::HumanReview, long).unwrap_err();
        assert!(err.contains("2048"));
    }

    #[test]
    fn approval_requirement_round_trips() {
        let r = ApprovalRequirement::new(
            fp(),
            ApprovalCategory::PolicyException,
            "skip Lintian `no-copyright-file` for binary-only upload",
        )
        .unwrap();
        let json = serde_json::to_string(&r).unwrap();
        let parsed: ApprovalRequirement = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn approval_registry_record_and_query() {
        let mut reg = ApprovalRegistry::new();
        let r = ApprovalRequirement::new(fp(), ApprovalCategory::Publication, "publish").unwrap();
        reg.record(r.id, ApprovalDecision::Approved);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get(r.id), Some(ApprovalDecision::Approved));
    }

    #[test]
    fn approval_registry_records_latest_decision() {
        let mut reg = ApprovalRegistry::new();
        let r = ApprovalRequirement::new(fp(), ApprovalCategory::Signing, "sign").unwrap();
        reg.record(r.id, ApprovalDecision::Approved);
        reg.record(r.id, ApprovalDecision::Rejected);
        assert_eq!(reg.get(r.id), Some(ApprovalDecision::Rejected));
    }

    #[test]
    fn approval_registry_iter_yields_decisions() {
        let mut reg = ApprovalRegistry::new();
        let r1 = ApprovalRequirement::new(fp(), ApprovalCategory::Publication, "p").unwrap();
        let r2 = ApprovalRequirement::new(fp(), ApprovalCategory::Signing, "s").unwrap();
        reg.record(r1.id, ApprovalDecision::Approved);
        reg.record(r2.id, ApprovalDecision::Rejected);
        let entries: Vec<_> = reg.iter().collect();
        assert_eq!(entries.len(), 2);
    }
}
