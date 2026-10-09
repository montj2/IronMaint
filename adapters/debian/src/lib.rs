//! `debian` — production DistributionAdapter for Debian (PHASE-1 §10).
//!
//! Implements [`DistributionFamily("debian")`] with the §11
//! "Expected" capability set: `SourceInspection`,
//! `VersionComparison`, `IssueRead`. Does **not** advertise any
//! Phase 2 capability (build / QA / issue-write / release /
//! publication); the `verify-seams` S6 check enforces this
//! structurally (1B.3).
//!
//! ## Module layout
//!
//!  - [`descriptor`] builds the static `AdapterDescriptor`.
//!  - [`versioning`] implements real dpkg §5.6.12 semantics
//!    (PHASE-1 §15) via the `debversion` crate.
//!  - [`package_model`] validates Debian package names per
//!    Policy §5.6.4 and classifies `debian/*` paths by role.
//!  - [`inspection`] implements `InspectionCapability` for
//!    Phase 1's first read-only check
//!    (`debian.inspect.source_preparation`, 1C.1).
//!  - [`issues`] implements `IssueCapability` for the
//!    `IssueRead`-only shape (the BTS transport is 1E.x).
//!
//! ## Why no `build` / `policy` / `release` modules
//!
//! Per §11, the production adapter does not advertise
//! `BuildPlanning`, `PolicyDerivation`, `ReleaseMetadata`, or
//! `PublicationPlanning`. Those sub-traits return `None` from
//! the corresponding accessors; no implementation exists.
//! Phase 2 is when these get wired in.

#![deny(unsafe_code)]
#![deny(unused_must_use)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod descriptor;
mod inspection;
mod issues;
mod package_model;
mod versioning;

use ironmaint_adapter_api::DistributionAdapter;

/// The production Debian adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebianAdapter;

impl DebianAdapter {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DistributionAdapter for DebianAdapter {
    fn descriptor(&self) -> ironmaint_adapter_api::AdapterDescriptor {
        descriptor::debian_descriptor().clone()
    }

    fn versioning(&self) -> &dyn ironmaint_adapter_api::VersioningCapability {
        versioning::debian_versioning()
    }

    fn package_model(&self) -> &dyn ironmaint_adapter_api::PackageModelCapability {
        package_model::debian_package_model()
    }

    fn policy(&self) -> Option<&dyn ironmaint_adapter_api::PolicyCapability> {
        // §11: `PolicyDerivation` is not advertised.
        None
    }

    fn build(&self) -> Option<&dyn ironmaint_adapter_api::BuildCapability> {
        // §11: `BuildPlanning` is not advertised.
        //
        // 1B.3 RED: the descriptor mistakenly includes
        // `BuildPlanning` (see `descriptor.rs`); the conformance
        // arm at `assert_descriptor_is_valid` therefore fails
        // with "advertised `BuildPlanning` but `build()` returns
        // `None`" until the GREEN commit removes the spurious
        // capability.
        None
    }

    fn issues(&self) -> Option<&dyn ironmaint_adapter_api::IssueCapability> {
        Some(issues::debian_issues())
    }

    fn release(&self) -> Option<&dyn ironmaint_adapter_api::ReleaseCapability> {
        // §11: `ReleaseMetadata` is not advertised.
        None
    }

    fn inspection(&self) -> Option<&dyn ironmaint_adapter_api::InspectionCapability> {
        Some(inspection::debian_inspection())
    }
}
