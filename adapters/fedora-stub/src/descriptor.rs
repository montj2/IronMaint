//! Fedora adapter descriptor (§45).
//!
//! The Fedora stub advertises three capabilities a Phase 2
//! packaging-only adapter exposes through the trait:
//! `SourceInspection`, `VersionComparison`, and `PackageQaPlanning`.
//!
//! Capabilities that have no production caller as of Phase 2 are
//! deliberately NOT advertised:
//!
//! - `BuildPlanning` — re-enabling is PR 2A.2's call.
//! - `FunctionalTestPlanning`, `UpgradeTestPlanning`,
//!   `ReproducibilityPlanning` — test-planning, not
//!   packaging.
//! - `IssueRead`, `IssueWrite` — out of Phase 2 scope for
//!   Fedora; the read path is Debian-only.
//! - `ReleaseMetadata`, `PublicationPlanning` — out of scope
//!   for Phase 2 per `doc/PHASE-2.md` §226 (no automatic
//!   signing/upload).
//! - `UpstreamDiscovery` — outside the packaging-only contract;
//!   PHASE-2 PR 2.0-E removes it from the stub set.
//! - `PolicyDerivation` — not yet wired into the Fedora
//!   policy plan; it will land in 1B.4.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterCapabilities, AdapterCapability, AdapterDescriptor};
use ironmaint_core::DistributionFamily;

static DESCRIPTOR: LazyLock<AdapterDescriptor> = LazyLock::new(build_descriptor);

fn build_descriptor() -> AdapterDescriptor {
    let mut caps = AdapterCapabilities::new();
    for c in [
        AdapterCapability::SourceInspection,
        AdapterCapability::VersionComparison,
        AdapterCapability::PackageQaPlanning,
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

/// Returns the static Fedora descriptor. Tests and consumers hold a
/// `&'static AdapterDescriptor`.
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
    fn capability_set_does_not_advertise_phase_2_no_callers() {
        // PHASE-2 PR 2.0-E: reconcile the stub's advertised
        // capability set with the runtime's actual callers.
        // The eight capabilities below have NO production
        // caller as of Phase 2.
        let d = fedora_descriptor();
        for removed in [
            AdapterCapability::UpstreamDiscovery,
            AdapterCapability::BuildPlanning,
            AdapterCapability::FunctionalTestPlanning,
            AdapterCapability::UpgradeTestPlanning,
            AdapterCapability::ReproducibilityPlanning,
            AdapterCapability::IssueRead,
            AdapterCapability::IssueWrite,
            AdapterCapability::ReleaseMetadata,
            AdapterCapability::PublicationPlanning,
        ] {
            assert!(
                !d.capabilities.contains(&removed),
                "{removed:?} must NOT be advertised by fedora-stub; it has no production caller"
            );
        }
    }

    #[test]
    fn descriptor_is_idempotent() {
        assert_eq!(fedora_descriptor(), fedora_descriptor());
    }
}
