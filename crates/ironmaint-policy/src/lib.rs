//! `ironmaint-policy` — policy, authority, approval, and privileged-operation model.
//!
//! Distribution-neutral. "It does not retrieve policy documents in
//! Phase 0A. It models them" (PHASE-0A.md §4.3). The crate may reference
//! [`ironmaint_core::EvidenceId`] and other core IDs as opaque
//! identifiers, but does NOT depend on `ironmaint-evidence` —
//! obligation types hold evidence IDs as opaque pointers, not as
//! evidence values, so the policy layer can be reasoned about
//! without loading the evidence crate.
//!
//! ## Phase 0A.3 status
//!
//! All value types are in place:
//! - [`Authority`], [`AuthorityClassification`] (§32)
//! - [`PolicyReference`], [`PolicyBaseline`] (§33, §36)
//! - [`ObligationStrength`], [`Applicability`], [`ObligationStatus`],
//!   [`Obligation`], [`ObligationSet`] (§34–§35)
//! - [`ApprovalRequirement`], [`ApprovalCategory`], [`ApprovalDecision`] (§40)
//! - [`AuthorizationState`], [`PrivilegedOperation`],
//!   [`PrivilegedOperationKind`] (§39, §41)
//!
//! Adapter-supplied [`Obligation`] instances, and policy evaluation,
//! land in `ironmaint-adapter-api` (0A.4).

#![forbid(unsafe_code)]
// Tests in this crate legitimately `.unwrap()` / `.expect()` on values
// whose constructors we've already validated by construction. The
// workspace `[lints.clippy]` table promotes `unwrap_used` and
// `expect_used` to `warn`, which combined with `-D warnings` would
// deny them in tests too. Allow them under `cfg(test)` only —
// production paths are still subject to the lint.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod approval;
pub mod authority;
pub mod issue_action;
pub mod obligation;
pub mod policy;
pub mod privileged;
pub mod release;

pub use approval::{ApprovalCategory, ApprovalDecision, ApprovalRegistry, ApprovalRequirement};
pub use authority::{Authority, AuthorityClassification};
pub use issue_action::{IssueAction, IssueActionKind};
pub use obligation::{
    Applicability, Obligation, ObligationSet, ObligationStatus, ObligationStrength,
};
pub use policy::{PolicyBaseline, PolicyReference};
pub use privileged::{AuthorizationState, PrivilegedOperation, PrivilegedOperationKind};
pub use release::{PublicationPlan, ReleaseCandidate};
