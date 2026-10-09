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
/// 1B.3 RED: `BuildPlanning` is in the capability set. The
/// `verify-seams` S6 check (added in the same RED commit) reports
/// `debian advertises BuildPlanning` against this file. The GREEN
/// commit deletes the `BuildPlanning` insertion; the seam check
/// goes green.
#[allow(clippy::expect_used, clippy::unwrap_used)]
fn build_descriptor() -> AdapterDescriptor {
    let mut caps = AdapterCapabilities::new();
    // §11 "Expected" — the real Phase 1 read-only-intake surface.
    caps.insert(AdapterCapability::SourceInspection);
    caps.insert(AdapterCapability::VersionComparison);
    caps.insert(AdapterCapability::IssueRead);
    // 1B.3 RED ONLY: this is the deliberate violation that
    // makes the broken-then-fixed pattern visible. The
    // GREEN commit deletes this line.
    caps.insert(AdapterCapability::BuildPlanning);

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
    fn descriptor_advertises_the_phase1_expected_set_plus_build_planning_red() {
        // 1B.3 RED: the descriptor advertises `BuildPlanning` in
        // addition to the §11 "Expected" set. The GREEN commit
        // removes `BuildPlanning`; this test changes name and
        // assertion in the same step.
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
        assert!(d.capabilities.contains(&AdapterCapability::BuildPlanning));
    }

    #[test]
    fn descriptor_is_idempotent() {
        // LazyLock returns the same instance; cloning yields equal
        // structures on every call.
        assert_eq!(debian_descriptor(), debian_descriptor());
    }
}
