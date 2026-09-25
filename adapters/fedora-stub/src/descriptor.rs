//! Fedora adapter descriptor (§45).
//!
//! Capability set mirrors the Debian stub minus SourceInspection and
//! PolicyDerivation, plus an explicit `SourceInspection` listing
//! (Fedora-side `rpm -i --dry-run` introspection). The cross-stub
//! acceptance test asserts descriptor families differ.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterCapabilities, AdapterCapability, AdapterDescriptor};
use ironmaint_core::DistributionFamily;

static DESCRIPTOR: LazyLock<AdapterDescriptor> = LazyLock::new(build_descriptor);

fn build_descriptor() -> AdapterDescriptor {
    let mut caps = AdapterCapabilities::new();
    for c in [
        AdapterCapability::SourceInspection,
        AdapterCapability::VersionComparison,
        AdapterCapability::UpstreamDiscovery,
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
        implementation_name: "fedora-stub".into(),
        implementation_version: "0.1.0".into(),
        capabilities: caps,
    }
}

#[allow(clippy::expect_used, clippy::unwrap_used)]
fn static_family() -> DistributionFamily {
    DistributionFamily::new("fedora")
        .expect("static literal \"fedora\" must satisfy DistributionFamily::new")
}

#[must_use]
pub fn fedora_descriptor() -> AdapterDescriptor {
    DESCRIPTOR.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_is_fedora() {
        let d = fedora_descriptor();
        assert_eq!(d.family.as_str(), "fedora");
    }

    #[test]
    fn implementation_name_is_fedora_stub() {
        let d = fedora_descriptor();
        assert_eq!(d.implementation_name, "fedora-stub");
    }

    #[test]
    fn capability_set_includes_source_inspection() {
        // Fedora advertises SourceInspection; Debian does not.
        let d = fedora_descriptor();
        assert!(
            d.capabilities
                .contains(&AdapterCapability::SourceInspection)
        );
    }

    #[test]
    fn descriptor_is_idempotent() {
        assert_eq!(fedora_descriptor(), fedora_descriptor());
    }
}
