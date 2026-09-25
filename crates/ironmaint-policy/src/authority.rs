//! Authorities — sources of policy (§32).
//!
//! An [`Authority`] is "where a policy assertion comes from". A
//! Debian Policy document, a Fedora Packaging Guidelines page, a
//! team-local style guide — each becomes an `Authority` with the
//! appropriate [`AuthorityClassification`]. Core doesn't rank them
//! (`§32: "Adapters provide ordering rules"`); it just stores them.

use std::fmt;

use serde::{Deserialize, Serialize};

use ironmaint_core::{AuthorityId, DistributionFamily};

/// Classification of an authority's normative weight (§32).
///
/// Adapter-supplied ranking rules apply *over* these labels; the
/// labels themselves are distribution-neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityClassification {
    /// Binding distribution policy (e.g. Debian Policy, Fedora
    /// Packaging Guidelines).
    NormativePolicy,
    /// Formal specification (RFC, ISO, etc.).
    FormalSpecification,
    /// Distribution-internal procedure document.
    DistributionProcedure,
    /// Industry best practice (not binding on the distribution).
    BestPractice,
    /// Tool documentation (lintian, rpmlint, sbuild, mock, etc.).
    ToolDocumentation,
    /// Distribution-team-specific policy.
    TeamPolicy,
    /// Local / project policy.
    LocalPolicy,
    /// Informational — does not assert any obligation.
    Informational,
}

impl fmt::Display for AuthorityClassification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl AuthorityClassification {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NormativePolicy => "normative_policy",
            Self::FormalSpecification => "formal_specification",
            Self::DistributionProcedure => "distribution_procedure",
            Self::BestPractice => "best_practice",
            Self::ToolDocumentation => "tool_documentation",
            Self::TeamPolicy => "team_policy",
            Self::LocalPolicy => "local_policy",
            Self::Informational => "informational",
        }
    }
}

/// A source of policy (§32).
///
/// `distribution` is `Some` for distribution-specific authorities
/// (Debian Policy, Fedora Packaging Guidelines) and `None` for
/// cross-distribution authorities (formal specs, generic best
/// practices). The classifier is distribution-neutral; ranking is
/// the adapter's job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Authority {
    pub id: AuthorityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<DistributionFamily>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub classification: AuthorityClassification,
}

impl Authority {
    #[must_use]
    pub fn new(
        distribution: Option<DistributionFamily>,
        name: impl Into<String>,
        classification: AuthorityClassification,
    ) -> Self {
        Self {
            id: AuthorityId::new(),
            distribution,
            name: name.into(),
            version: None,
            classification,
        }
    }

    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&AuthorityClassification::NormativePolicy).unwrap(),
            r#""normative_policy""#
        );
        assert_eq!(
            serde_json::to_string(&AuthorityClassification::FormalSpecification).unwrap(),
            r#""formal_specification""#
        );
    }

    #[test]
    fn classification_display_is_snake_case() {
        assert_eq!(
            AuthorityClassification::DistributionProcedure.to_string(),
            "distribution_procedure"
        );
    }

    #[test]
    fn authority_minimum_construction() {
        let a = Authority::new(
            None,
            "RFC 4180",
            AuthorityClassification::FormalSpecification,
        );
        assert!(a.distribution.is_none());
        assert_eq!(a.name, "RFC 4180");
        assert!(a.version.is_none());
    }

    #[test]
    fn authority_with_distribution() {
        let family = DistributionFamily::new("debian").unwrap();
        let a = Authority::new(
            Some(family),
            "Debian Policy",
            AuthorityClassification::NormativePolicy,
        )
        .with_version("4.7.4.1");
        assert_eq!(a.distribution.as_ref().unwrap().as_str(), "debian");
        assert_eq!(a.version.as_deref(), Some("4.7.4.1"));
    }

    #[test]
    fn authority_round_trips() {
        let a = Authority::new(
            Some(DistributionFamily::new("fedora").unwrap()),
            "Fedora Packaging Guidelines",
            AuthorityClassification::NormativePolicy,
        )
        .with_version("main");
        let json = serde_json::to_string(&a).unwrap();
        let parsed: Authority = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, a);
    }

    #[test]
    fn authority_omits_none_distribution_and_version() {
        let a = Authority::new(
            None,
            "Generic Best Practices",
            AuthorityClassification::BestPractice,
        );
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("distribution"));
        assert!(!json.contains("version"));
    }
}
