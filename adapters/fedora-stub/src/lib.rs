//! `fedora-stub` — Phase 0A stub adapter for Fedora.
//!
//! Implements [`DistributionAdapter`] for `DistributionFamily("fedora")`.
//! Real RPM EVR comparison, Fedora guideline retrieval, and
//! `rpmlint`/`Mock`/`Koji`/`Bodhi` integration are explicitly out of scope
//! for Phase 0A (PHASE-0A.md §89).
//!
//! ## Phase 0A.4 status
//!
//! All six capability traits implemented and visibly different from the
//! Debian stub. Cross-stub acceptance tests live in
//! `crates/ironmaint-adapter-api/tests/`.
//!
//! ## Module layout
//!
//! - [`descriptor`] builds the static `AdapterDescriptor`.
//! - [`versioning`] implements Fedora version split `epoch:version-release`
//!   with `.fcNN`/`.elNN` tag stripping.
//! - [`package_model`] classifies `*.spec`, `sources`, `%files`-prefixed
//!   paths, validates package names per Fedora RPM grammar.
//! - [`policy_cap`] orders authorities with `LocalPolicy` between
//!   `DistributionProcedure` and `FormalSpecification`.
//! - [`build`] emits `fedora.build.mock` + `fedora.qa.*` plans.
//! - [`issues`] advertises the Bugzilla provider id, supports
//!   `SetSeverity / Reopen / AddLabels / RemoveLabels / Comment`.
//! - [`release`] plans `fedora/release` metadata + push + koji submit +
//!   bodhi update.

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

/// Fedora stub adapter orchestrator (§45).
#[derive(Debug, Default, Clone, Copy)]
pub struct FedoraStubAdapter;

impl FedoraStubAdapter {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DistributionAdapter for FedoraStubAdapter {
    fn descriptor(&self) -> ironmaint_adapter_api::AdapterDescriptor {
        descriptor::fedora_descriptor()
    }

    fn versioning(&self) -> &dyn VersioningCapability {
        versioning::fedora_versioning()
    }

    fn package_model(&self) -> &dyn PackageModelCapability {
        package_model::fedora_package_model()
    }

    fn policy(&self) -> Option<&dyn PolicyCapability> {
        Some(policy_cap::fedora_policy())
    }

    fn build(&self) -> Option<&dyn BuildCapability> {
        Some(build::fedora_build())
    }

    fn issues(&self) -> Option<&dyn IssueCapability> {
        Some(issues::fedora_issues())
    }

    fn release(&self) -> Option<&dyn ReleaseCapability> {
        Some(release::fedora_release())
    }
}
