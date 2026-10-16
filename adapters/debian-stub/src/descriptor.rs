//! Debian adapter descriptor (§45).
//!
//! The Debian stub advertises the four capabilities a Phase 2
//! packaging-only adapter exposes through the trait:
//! `VersionComparison`, `PolicyDerivation`, `PackageQaPlanning`,
//! and `IssueRead`. `BuildPlanning` is not advertised —
//! re-enabling it is PR 2A.2's call (it must round-trip a real
//! `BuildPlan`).
//!
//! Capabilities that have no production caller as of Phase 2 are
//! deliberately NOT advertised:
//!
//! - `FunctionalTestPlanning`, `UpgradeTestPlanning`,
//!   `ReproducibilityPlanning` — test-planning, not
//!   packaging.
//! - `IssueWrite` — only `IssueRead` is needed for the Phase 2
//!   policy-evaluation read path.
//! - `ReleaseMetadata`, `PublicationPlanning` — out of scope
//!   for Phase 2 per `doc/PHASE-2.md` §226 (no automatic
//!   signing/upload).
//! - `SourceInspection` and `UpstreamDiscovery` — outside the
//!   packaging-only contract.
//!
//! The conformance arm in `ironmaint-testkit` enforces the
//! descriptor ↔ trait-method cross-check: any capability the
//! stubs advertise must have a matching `Some(...)` on the
//! trait. The trimmed set listed above is the complete
//! production-pinned set for Phase 2.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterCapabilities, AdapterCapability, AdapterDescriptor};
use ironmaint_core::DistributionFamily;

static DESCRIPTOR: LazyLock<AdapterDescriptor> = LazyLock::new(build_descriptor);

fn build_descriptor() -> AdapterDescriptor {
    let mut caps = AdapterCapabilities::new();
    for c in [
        AdapterCapability::VersionComparison,
        AdapterCapability::PolicyDerivation,
        AdapterCapability::PackageQaPlanning,
        AdapterCapability::IssueRead,
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
        assert!(names.contains(&"policy_derivation"));
        assert!(names.contains(&"package_qa_planning"));
        assert!(names.contains(&"issue_read"));
    }

    #[test]
    fn capability_set_does_not_advertise_phase_2_no_callers() {
        // PHASE-2 PR 2.0-E: reconcile the stub's advertised
        // capability set with the runtime's actual callers.
        // The seven capabilities below have NO production
        // caller as of Phase 2 — `cargo run -p xtask --
        // verify-architecture` cannot forbid them on stubs
        // (the S6 seam check constrains production
        // adapters), so the stubs trim them by hand.
        let d = debian_descriptor();
        for removed in [
            AdapterCapability::BuildPlanning,
            AdapterCapability::FunctionalTestPlanning,
            AdapterCapability::UpgradeTestPlanning,
            AdapterCapability::ReproducibilityPlanning,
            AdapterCapability::IssueWrite,
            AdapterCapability::ReleaseMetadata,
            AdapterCapability::PublicationPlanning,
        ] {
            assert!(
                !d.capabilities.contains(&removed),
                "{removed:?} must NOT be advertised by debian-stub; it has no production caller"
            );
        }
    }

    #[test]
    fn descriptor_is_idempotent() {
        // LazyLock returns the same instance; cloning yields equal
        // structures on every call.
        assert_eq!(debian_descriptor(), debian_descriptor());
    }
}
