//! Production Debian adapter descriptor (PHASE-1 §10, §11).
//!
//! 1B.3 RED: this descriptor intentionally advertises
//! [`AdapterCapability::BuildPlanning`] alongside the §11 "Expected"
//! set. The `verify-seams` S6 check (also added in 1B.3 RED) will
//! fire on this; the GREEN commit removes `BuildPlanning` and
//! the seam check goes green.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterCapabilities, AdapterCapability, AdapterDescriptor};
use ironmaint_core::DistributionFamily;

/// The production adapter's static descriptor.
///
/// 1B.3 GREEN: the capability set is exactly §11 "Expected" —
/// no Phase 2 capabilities are advertised. The S6 seam check
/// (`xtask/src/seams/adapters.rs`) is the structural guard.
#[allow(clippy::expect_used, clippy::unwrap_used)]
fn build_descriptor() -> AdapterDescriptor {
    let mut caps = AdapterCapabilities::new();
    // §11 "Expected" — the real Phase 1 read-only-intake surface.
    caps.insert(AdapterCapability::SourceInspection);
    caps.insert(AdapterCapability::VersionComparison);
    caps.insert(AdapterCapability::IssueRead);

    AdapterDescriptor {
        family: DistributionFamily::new("debian")
            .expect("static literal \"debian\" must satisfy DistributionFamily::new"),
        implementation_name: "debian".into(),
        implementation_version: "0.1.0".into(),
        capabilities: caps,
    }
}

/// The lazy static singleton — every `descriptor()` call returns a
/// reference to the same `AdapterDescriptor`.
#[must_use]
pub fn debian_descriptor() -> &'static AdapterDescriptor {
    static INSTANCE: LazyLock<AdapterDescriptor> = LazyLock::new(build_descriptor);
    &INSTANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_advertises_the_phase1_expected_set_only() {
        // 1B.3 GREEN: the descriptor advertises exactly the §11
        // "Expected" set. The S6 seam check is the structural
        // guard against re-introducing a Phase 2 capability
        // in a future PR.
        let d = debian_descriptor();
        assert!(
            d.capabilities
                .contains(&AdapterCapability::SourceInspection)
        );
        assert!(
            d.capabilities
                .contains(&AdapterCapability::VersionComparison)
        );
        assert!(d.capabilities.contains(&AdapterCapability::IssueRead));
        assert!(!d.capabilities.contains(&AdapterCapability::BuildPlanning));
        assert!(
            !d.capabilities
                .contains(&AdapterCapability::PackageQaPlanning)
        );
        assert!(!d.capabilities.contains(&AdapterCapability::IssueWrite));
        assert!(!d.capabilities.contains(&AdapterCapability::ReleaseMetadata));
        assert!(
            !d.capabilities
                .contains(&AdapterCapability::PublicationPlanning)
        );
    }

    #[test]
    fn descriptor_is_idempotent() {
        // LazyLock returns the same instance; cloning yields equal
        // structures on every call.
        assert_eq!(debian_descriptor(), debian_descriptor());
    }
}
