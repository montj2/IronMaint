//! Adapter self-description and capability declarations (§44).
//!
//! `AdapterDescriptor` is what every adapter returns to announce
//! itself: which distribution, which implementation, and which
//! capabilities the adapter claims. The capability set is a
//! declarative list (not a behavioral guarantee) — the
//! `DistributionAdapter` impl is responsible for agreeing with the
//! descriptor (e.g. an adapter advertising `BuildPlanning` must
//! return `Some(...)` from `DistributionAdapter::build()`).

use std::collections::BTreeSet;

use ironmaint_core::DistributionFamily;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Self-description of an adapter (§44).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdapterDescriptor {
    pub family: DistributionFamily,
    pub implementation_name: String,
    pub implementation_version: String,
    pub capabilities: AdapterCapabilities,
}

/// Set of [`AdapterCapability`] values the adapter claims (§44).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(transparent)]
pub struct AdapterCapabilities {
    inner: BTreeSet<AdapterCapability>,
}

impl AdapterCapabilities {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a capability. Returns `true` if it was not already
    /// present.
    pub fn insert(&mut self, capability: AdapterCapability) -> bool {
        self.inner.insert(capability)
    }

    #[must_use]
    pub fn contains(&self, capability: &AdapterCapability) -> bool {
        self.inner.contains(capability)
    }

    pub fn iter(&self) -> impl Iterator<Item = &AdapterCapability> {
        self.inner.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl FromIterator<AdapterCapability> for AdapterCapabilities {
    fn from_iter<I: IntoIterator<Item = AdapterCapability>>(iter: I) -> Self {
        Self {
            inner: iter.into_iter().collect(),
        }
    }
}

/// Declarative capability enumeration (§44).
///
/// The 13 variants exactly match the spec list. Adding a variant is a
/// spec change; this enum is `non_exhaustive` so future extensions
/// don't break downstream adapters.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AdapterCapability {
    /// Inspect upstream VCS source: clone, walk refs, enumerate commits.
    SourceInspection,
    /// Validate and compare package versions for this distribution family.
    VersionComparison,
    /// Discover upstream releases (monitor tags, security feeds).
    UpstreamDiscovery,
    /// Derive [`PolicyPlan`]s and obligation templates.
    PolicyDerivation,
    /// Produce [`BuildPlan`]s describing build gates.
    BuildPlanning,
    /// Produce [`QaPlan`]s describing QA gates.
    PackageQaPlanning,
    /// Plan functional-test runs.
    FunctionalTestPlanning,
    /// Plan upgrade-test runs (existing-install path).
    UpgradeTestPlanning,
    /// Plan reproducibility-test runs.
    ReproducibilityPlanning,
    /// Read issue tracker state (BTS / Bugzilla).
    IssueRead,
    /// Mutate issue tracker state (comment, tag, attach).
    IssueWrite,
    /// Produce release-metadata plans (tags, notes, changelog entries).
    ReleaseMetadata,
    /// Produce [`PublicationPlanTemplate`]s with privileged operations.
    PublicationPlanning,
}

impl AdapterCapability {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SourceInspection => "source_inspection",
            Self::VersionComparison => "version_comparison",
            Self::UpstreamDiscovery => "upstream_discovery",
            Self::PolicyDerivation => "policy_derivation",
            Self::BuildPlanning => "build_planning",
            Self::PackageQaPlanning => "package_qa_planning",
            Self::FunctionalTestPlanning => "functional_test_planning",
            Self::UpgradeTestPlanning => "upgrade_test_planning",
            Self::ReproducibilityPlanning => "reproducibility_planning",
            Self::IssueRead => "issue_read",
            Self::IssueWrite => "issue_write",
            Self::ReleaseMetadata => "release_metadata",
            Self::PublicationPlanning => "publication_planning",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::DistributionFamily;

    fn family() -> DistributionFamily {
        DistributionFamily::new("debian").unwrap()
    }

    #[test]
    fn descriptor_minimum() {
        let d = AdapterDescriptor {
            family: family(),
            implementation_name: "debian-stub".into(),
            implementation_version: "0.1.0".into(),
            capabilities: AdapterCapabilities::new(),
        };
        assert_eq!(d.family.as_str(), "debian");
        assert!(d.capabilities.is_empty());
    }

    #[test]
    fn capabilities_set_semantics() {
        let mut caps = AdapterCapabilities::new();
        assert!(caps.insert(AdapterCapability::VersionComparison));
        assert!(!caps.insert(AdapterCapability::VersionComparison));
        assert!(caps.contains(&AdapterCapability::VersionComparison));
        assert!(!caps.is_empty());
        assert_eq!(caps.len(), 1);
    }

    #[test]
    fn capabilities_iteration_is_ordered() {
        let caps: AdapterCapabilities = [
            AdapterCapability::IssueRead,
            AdapterCapability::BuildPlanning,
            AdapterCapability::SourceInspection,
        ]
        .into_iter()
        .collect();
        let names: Vec<&str> = caps.iter().map(|c| c.name()).collect();
        // BTreeSet orders by the enum's derived Ord (declaration order).
        assert_eq!(
            names,
            vec!["source_inspection", "build_planning", "issue_read"]
        );
    }

    #[test]
    fn capability_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&AdapterCapability::PackageQaPlanning).unwrap(),
            r#""package_qa_planning""#
        );
    }

    #[test]
    fn descriptor_round_trips() {
        let d = AdapterDescriptor {
            family: family(),
            implementation_name: "x".into(),
            implementation_version: "0.1.0".into(),
            capabilities: AdapterCapabilities::new(),
        };
        let json = serde_json::to_string(&d).unwrap();
        let parsed: AdapterDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, d);
    }
}
