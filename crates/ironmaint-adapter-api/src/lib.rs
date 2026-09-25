//! `ironmaint-adapter-api` — the adapter contract every distribution
//! implements (PHASE-0A.md §45–§54).
//!
//! Adapters implement the [`DistributionAdapter`] root trait plus the
//! capability-specific traits ([`VersioningCapability`],
//! [`PackageModelCapability`], [`PolicyCapability`], [`BuildCapability`],
//! [`IssueCapability`], [`ReleaseCapability`]). The root trait returns
//! `&dyn Capability` references so consumers can dispatch through the
//! trait objects without knowing the concrete adapter.
//!
//! ## Dependency direction
//!
//! Depends on `ironmaint-core`, `ironmaint-evidence`, and
//! `ironmaint-policy`. Per PHASE-0A.md §4.5, this crate "should avoid
//! depending directly on `ironmaint-state`" — and it doesn't.
//!
//! ## Phase 0A.4 status
//!
//! Full contract surface lands in commit 1 (this commit). Concrete
//! Debian and Fedora stub implementations land in commits 2 and 3.

#![forbid(unsafe_code)]
// Tests legitimately `.unwrap()` / `.expect()` on validated inputs.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod build;
pub mod contexts;
pub mod descriptor;
pub mod error;
pub mod issues;
pub mod package_model;
pub mod policy_port;
pub mod release;
pub mod tool_key;
pub mod versioning;

pub use build::{BuildCapability, BuildPlan, PlannedCheck, QaPlan};
pub use contexts::{CandidateContext, PolicyContext, PublicationContext, ReleaseContext};
pub use descriptor::{AdapterCapabilities, AdapterCapability, AdapterDescriptor};
pub use error::{AdapterError, AdapterErrorKind};
pub use issues::{IssueCapability, IssuePlan};
pub use package_model::{ChangedPath, PackageModelCapability, PathRole};
pub use policy_port::{ObligationTemplate, PolicyCapability, PolicyPlan};
pub use release::{
    PlannedOperation, PublicationPlanTemplate, ReleaseCapability, ReleaseMetadataPlan,
};
pub use tool_key::ToolCapabilityKey;
pub use versioning::VersioningCapability;

/// Root adapter trait (§45).
///
/// All capability methods are `&self`-only and synchronous. Adapters
/// never touch workflow state — the engine is the only writer
/// (PHASE-0A.md §2.5, §4.4, §18).
///
/// Capabilities other than `versioning` and `package_model` return
/// `Option<&dyn …>` because some adapters may not implement them
/// (e.g. a metadata-only adapter has no build tooling).
///
/// Object safety requires `Send + Sync + 'static` and the absence of
/// generic methods. The `tests/capability_object_safety.rs`
/// integration test asserts every capability trait is dyn-compatible.
/// Distribution-shaped adapter contract (§44).
///
/// `DistributionAdapter` is the only surface adapters expose to
/// IronMaint core. Adapters describe what *must* happen; the core
/// decides whether it does. Concretely:
///
/// - **Represents**: a deterministic, distribution-specific
///   description of how to inspect, plan, and gate a package workflow
///   for one distribution family.
/// - **Mutation ownership**: adapters MUST NOT mutate `JobState`,
///   `GateStatus`, `ObligationStatus`, `ApprovalStatus`, or
///   `PublicationStatus`. The state machine is owned by
///   `ironmaint_state::TransitionEngine`. Adapters also MUST NOT
///   mutate `AuthorizationState`; only the privileged service may
///   do that.
/// - **Invariants**: the trait is `Send + Sync + 'static` so adapters
///   are shareable across threads. The capability set returned by
///   [`descriptor`](Self::descriptor) MUST match the set of
///   `Some(&dyn ...)` returns from [`policy`](Self::policy),
///   [`build`](Self::build), [`issues`](Self::issues), and
///   [`release`](Self::release); the conformance suite in
///   `ironmaint_testkit::conformance` enforces this.
/// - **Does NOT represent**: this trait is NOT a workflow executor,
///   NOT a state machine, NOT a privileged side-effect initiator,
///   and NOT a remote-call surface. Adapters produce plans; they do
///   not execute plans.
pub trait DistributionAdapter: Send + Sync + 'static {
    /// Self-description of this adapter (§44). Returned every call;
    /// must be cheap and side-effect-free.
    fn descriptor(&self) -> AdapterDescriptor;

    /// Versioning capability (§47). MUST be implemented by every
    /// adapter; it owns version-string validation and comparison for
    /// its distribution family.
    fn versioning(&self) -> &dyn VersioningCapability;
    /// Package model capability (§48). MUST be implemented by every
    /// adapter; it owns package-name validation and change-domain
    /// classification.
    fn package_model(&self) -> &dyn PackageModelCapability;

    /// Policy derivation capability (§46). Returned only when the
    /// adapter advertises [`AdapterCapability::PolicyDerivation`]; if
    /// advertised, this MUST return `Some`.
    fn policy(&self) -> Option<&dyn PolicyCapability>;
    /// Build / QA planning capability (§49). Returned only when the
    /// adapter advertises [`AdapterCapability::BuildPlanning`] or
    /// [`AdapterCapability::PackageQaPlanning`].
    fn build(&self) -> Option<&dyn BuildCapability>;
    /// Issue-tracker capability (§51). Returned only when the
    /// adapter advertises [`AdapterCapability::IssueRead`] or
    /// [`AdapterCapability::IssueWrite`].
    fn issues(&self) -> Option<&dyn IssueCapability>;
    /// Release-metadata and publication capability (§§52-53).
    /// Returned only when the adapter advertises
    /// [`AdapterCapability::ReleaseMetadata`] or
    /// [`AdapterCapability::PublicationPlanning`].
    fn release(&self) -> Option<&dyn ReleaseCapability>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn DistributionAdapter>();
    }
}
