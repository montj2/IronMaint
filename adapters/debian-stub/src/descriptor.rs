//! Debian adapter descriptor (§45).
//!
//! The Debian stub advertises the capabilities §44 lists as belonging
//! to a packaging-only adapter: `VersionComparison`,
//! `PackageQaPlanning`, `BuildPlanning`, `IssueRead`, `IssueWrite`,
//! `ReleaseMetadata`, `PublicationPlanning`. Source inspection and
//! upstream discovery are deliberately absent — the Phase 0A.5
//! conformance suite will use that absence to assert the consumer
//! reads capabilities from the descriptor, not from a static
//! switch on family name.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterCapabilities, AdapterCapability, AdapterDescriptor};
use ironmaint_core::DistributionFamily;

static DESCRIPTOR: LazyLock<AdapterDescriptor> = LazyLock::new(build_descriptor);

fn build_descriptor() -> AdapterDescriptor {
    let mut caps = AdapterCapabilities::new();
    for c in [
        AdapterCapability::VersionComparison,
        AdapterCapability::PolicyDerivation,
        AdapterCapability::BuildPlanning,
        AdapterCapability::PackageQaPlanning,
        AdapterCapability::FunctionalTestPlanning,
        AdapterCapability::UpgradeTestPlanning,
        AdapterCapability::ReproducibilityPlanning,
        AdapterCapability::IssueRead,
        AdapterCapability::IssueWrite,
        AdapterCapability::ReleaseMetadata,
        AdapterCapability::PublicationPlanning,
    ] {
        caps.insert(c);
    }
    AdapterDescriptor {
        family: static_family(),
        implementation_name: "debian-stub".into(),
        implementation_version: "0.1.0".into(),
        capabilities: caps,
    }
}

/// Construct the [`DistributionFamily`] from a static literal.
///
/// The literal `"debian"` satisfies the
/// [`DistributionFamily::new`] grammar by construction — this is a
/// compile-time invariant, not a runtime check. We narrow the
/// `expect_used` allow to this one helper to keep the panic
/// behaviour localised.
#[allow(clippy::expect_used, clippy::unwrap_used)]
fn static_family() -> DistributionFamily {
    DistributionFamily::new("debian")
        .expect("static literal \"debian\" must satisfy DistributionFamily::new")
}

/// Returns the static Debian descriptor. Tests and consumers hold a
/// `&'static AdapterDescriptor`.
#[must_use]
pub fn debian_descriptor() -> AdapterDescriptor {
    DESCRIPTOR.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_is_debian() {
        let d = debian_descriptor();
        assert_eq!(d.family.as_str(), "debian");
    }

    #[test]
    fn implementation_name_is_debian_stub() {
        let d = debian_descriptor();
        assert_eq!(d.implementation_name, "debian-stub");
    }

    #[test]
    fn capability_set_is_complete() {
        let d = debian_descriptor();
        let names: Vec<&str> = d.capabilities.iter().map(AdapterCapability::name).collect();
        assert!(names.contains(&"version_comparison"));
        assert!(names.contains(&"build_planning"));
        assert!(names.contains(&"package_qa_planning"));
        assert!(names.contains(&"issue_read"));
        assert!(names.contains(&"issue_write"));
        assert!(names.contains(&"publication_planning"));
    }

    #[test]
    fn descriptor_is_idempotent() {
        // LazyLock returns the same instance; cloning yields equal
        // structures on every call.
        assert_eq!(debian_descriptor(), debian_descriptor());
    }
}
