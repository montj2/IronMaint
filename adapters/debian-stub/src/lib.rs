//! `debian-stub` — Phase 0A stub adapter for Debian.
//!
//! Implements [`DistributionAdapter`] for `DistributionFamily("debian")`.
//! The real `dpkg --compare-versions` semantics, Debian Policy
//! retrieval, and `lintian`/`sbuild` integration are explicitly out of
//! scope for Phase 0A (PHASE-0A.md §89).
//!
//! ## Phase 0A.4 status
//!
//! All six capability traits are implemented and visibly different
//! from the Fedora stub (see `crates/ironmaint-adapter-api/tests/`
//! for the cross-stub acceptance suite landing in commit 3).
//!
//! ## Module layout
//!
//! - [`descriptor`] builds the static `AdapterDescriptor`.
//! - [`versioning`] implements Debian version split
//!   `epoch:upstream-debian_revision`.
//! - [`package_model`] classifies `debian/*` paths and validates
//!   package names per Debian Policy §5.6.4.
//! - [`policy_cap`] orders authorities (Debian-specific ordering) and
//!   derives the Debian Policy / Developer's Reference obligation set.
//! - [`build`] emits `debian.build.sbuild` and `debian.qa.*` plans.
//! - [`issues`] advertises the BTS provider id and validates BTS ids.
//! - [`release`] plans `debian/release` metadata + push + BTS tagging.
//! - [`fixtures`] (test-only) builds shared contexts.

#![forbid(unsafe_code)]
// Tests legitimately `.unwrap()` / `.expect()` on validated inputs.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod build;
pub mod descriptor;
pub mod issues;
pub mod package_model;
pub mod policy_cap;
pub mod release;
pub mod versioning;

#[cfg(test)]
mod fixtures;

use ironmaint_adapter_api::{
    BuildCapability, DistributionAdapter, IssueCapability, PackageModelCapability,
    PolicyCapability, ReleaseCapability, VersioningCapability,
};

/// Debian stub adapter orchestrator (§45).
///
/// Zero-sized — all state lives in the [`LazyLock`] singletons inside
/// the capability modules. Cloning a `DebianStubAdapter` yields a
/// fresh handle to the same underlying singletons.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebianStubAdapter;

impl DebianStubAdapter {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DistributionAdapter for DebianStubAdapter {
    fn descriptor(&self) -> ironmaint_adapter_api::AdapterDescriptor {
        descriptor::debian_descriptor()
    }

    fn versioning(&self) -> &dyn VersioningCapability {
        versioning::debian_versioning()
    }

    fn package_model(&self) -> &dyn PackageModelCapability {
        package_model::debian_package_model()
    }

    fn policy(&self) -> Option<&dyn PolicyCapability> {
        Some(policy_cap::debian_policy())
    }

    fn build(&self) -> Option<&dyn BuildCapability> {
        Some(build::debian_build())
    }

    fn issues(&self) -> Option<&dyn IssueCapability> {
        Some(issues::debian_issues())
    }

    fn release(&self) -> Option<&dyn ReleaseCapability> {
        Some(release::debian_release())
    }
}
