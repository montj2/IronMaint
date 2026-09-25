//! Invalidation model (§31).
//!
//! PHASE-0A.md §31: "Define the structure now even though sophisticated
//! invalidation arrives later." [`InvalidationRule`] declares "if X
//! changes in domain Y, invalidate evidence of kind Z". We store them
//! and resolve them mechanically; sophisticated propagation lands in
//! 0A.5+.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::EvidenceId;

use crate::evidence::EvidenceKind;

/// Generic change domain (§31).
///
/// Adapters may supply additional rules under [`Self::AdapterSpecific`].
/// Core itself must not understand `debian/control` or `%files`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeDomain {
    UpstreamSource,
    PackagingMetadata,
    BuildConfiguration,
    RuntimeDependencies,
    Tests,
    PatchSet,
    LicensingMetadata,
    DocumentationOnly,
    ReleaseMetadata,
    /// Adapter-supplied change domain (e.g. `debian-control`).
    AdapterSpecific(String),
}

/// A rule: "if the change domain is X, invalidate evidence of these
/// kinds" (§31).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvalidationRule {
    pub change_domain: ChangeDomain,
    pub invalidates: Vec<EvidenceKind>,
}

impl InvalidationRule {
    #[must_use]
    pub fn new(change_domain: ChangeDomain, invalidates: Vec<EvidenceKind>) -> Self {
        Self {
            change_domain,
            invalidates,
        }
    }

    /// Apply this rule: return the evidence kinds it says to invalidate.
    #[must_use]
    pub fn invalidates_for(&self) -> &[EvidenceKind] {
        &self.invalidates
    }
}

/// The specific change that triggered invalidation.
///
/// The state machine and the conformance suite will eventually produce
/// these; 0A.3 only stores them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvalidationCause {
    pub domain: ChangeDomain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggering_evidence: Option<EvidenceId>,
    #[serde(with = "time::serde::rfc3339")]
    pub detected_at: OffsetDateTime,
}

impl InvalidationCause {
    #[must_use]
    pub fn new(domain: ChangeDomain, detected_at: OffsetDateTime) -> Self {
        Self {
            domain,
            triggering_evidence: None,
            detected_at,
        }
    }

    #[must_use]
    pub fn with_triggering_evidence(mut self, id: EvidenceId) -> Self {
        self.triggering_evidence = Some(id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn change_domain_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ChangeDomain::UpstreamSource).unwrap(),
            r#""upstream_source""#
        );
        assert_eq!(
            serde_json::to_string(&ChangeDomain::RuntimeDependencies).unwrap(),
            r#""runtime_dependencies""#
        );
    }

    #[test]
    fn change_domain_adapter_specific_carries_string() {
        let d = ChangeDomain::AdapterSpecific("debian-control".to_string());
        let json = serde_json::to_string(&d).unwrap();
        let parsed: ChangeDomain = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn invalidation_rule_stores_and_returns_kinds() {
        let rule = InvalidationRule::new(
            ChangeDomain::PackagingMetadata,
            vec![EvidenceKind::Build, EvidenceKind::PackageQa],
        );
        assert_eq!(rule.invalidates.len(), 2);
        assert_eq!(rule.invalidates_for()[0], EvidenceKind::Build);
    }

    #[test]
    fn invalidation_cause_new_has_no_triggering_evidence() {
        let cause = InvalidationCause::new(
            ChangeDomain::UpstreamSource,
            datetime!(2026-01-01 00:00:00 UTC),
        );
        assert!(cause.triggering_evidence.is_none());
    }

    #[test]
    fn invalidation_cause_with_triggering_evidence() {
        let cause = InvalidationCause::new(
            ChangeDomain::UpstreamSource,
            datetime!(2026-01-01 00:00:00 UTC),
        )
        .with_triggering_evidence(EvidenceId::new());
        assert!(cause.triggering_evidence.is_some());
    }

    #[test]
    fn invalidation_rule_round_trips() {
        let rule = InvalidationRule::new(
            ChangeDomain::BuildConfiguration,
            vec![EvidenceKind::Reproducibility, EvidenceKind::Build],
        );
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: InvalidationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rule);
    }

    #[test]
    fn invalidation_cause_round_trips() {
        let cause = InvalidationCause::new(ChangeDomain::Tests, datetime!(2026-01-01 00:00:00 UTC))
            .with_triggering_evidence(EvidenceId::new());
        let json = serde_json::to_string(&cause).unwrap();
        let parsed: InvalidationCause = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cause);
    }

    #[test]
    fn invalidation_cause_omits_none_triggering_evidence() {
        let cause = InvalidationCause::new(
            ChangeDomain::DocumentationOnly,
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&cause).unwrap();
        assert!(!json.contains("triggering_evidence"));
    }
}
