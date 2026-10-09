//! Read-only source-intake capability (PHASE-1.md §12).
//!
//! The `InspectionCapability` sub-trait is the read-only sibling of
//! `BuildCapability`. It produces the set of `PlannedCheck` records
//! the runtime should materialise on candidate capture, the same
//! way `build_plan` and `qa_plan` contribute to the merged plan
//! during `handle_capture_candidate` (see
//! `ironmaint-runtime::service::plan_checks`).
//!
//! ## Why a separate sub-trait
//!
//! The state machine treats source intake differently from build /
//! QA: intake happens at the moment the candidate is captured
//! (before any tool runs), and its checks produce `SourcePreparation`
//! / `SourceIntegrity` / `IssueCorrelation` / `MaintenanceContext`
//! evidence kinds (`gate_stage_for` maps them to the right gates).
//! Build and QA plans contribute later and live under different
//! gates. Folding the inspection shape into `BuildPlan` would have
//! hidden that lifecycle boundary; the sub-trait surfaces it.
//!
//! ## Wired in 1B.1
//!
//! `DistributionAdapter::inspection()` returns `Option<&dyn
//! InspectionCapability>`; adapters that do not advertise
//! [`AdapterCapability::SourceInspection`] return `None`. The
//! `assert_descriptor_is_valid` conformance arm in
//! `ironmaint_testkit::conformance` enforces the
//! "advertise-then-implement" rule for this capability, matching
//! the rule already in force for `policy()`, `build()`, `issues()`,
//! and `release()`.
//!
//! ## Out of scope
//!
//! The real Debian inspection content (`debian.inspect.source_preparation`,
//! `debian.inspect.source_analysis`, etc., per §13) lands in 1C.x.
//! 1B.1 ships only the *trait surface* and a hermetic no-op
//! implementation in the Fedora stub so the conformance suite can
//! walk it.

use crate::build::PlannedCheck;
use crate::contexts::CandidateContext;
use crate::error::AdapterError;

/// The set of read-only source-intake checks an adapter plans for a
/// candidate.
///
/// `checks` are flattened into the same tuple stream that
/// `BuildPlan::checks` and `QaPlan::checks` already feed; the
/// runtime does not distinguish "this came from `inspection_plan`"
/// at the materialise step. The evidence-kind field on each
/// `PlannedCheck` is what puts the check on the right gate.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InspectionPlan {
    pub checks: Vec<PlannedCheck>,
}

impl InspectionPlan {
    /// Empty plan. Adapters whose source intake is a no-op return
    /// this; the runtime treats it identically to "no inspection".
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }
}

/// The read-only source-intake capability sub-trait.
///
/// Implementors produce a list of `PlannedCheck` records the
/// runtime materialises at candidate capture. The capability
/// returns *only* plans — no I/O, no side effects, no
/// distribution of its own (§12, "The runtime aggregates
/// inspection checks during candidate capture exactly as it
/// already aggregates build/QA checks").
///
/// Object-safety: the `tests/capability_object_safety.rs`
/// integration test asserts the trait is dyn-compatible; the
/// `Send + Sync + 'static` bound is what `DistributionAdapter`
/// requires of every capability sub-trait.
pub trait InspectionCapability: Send + Sync + 'static {
    /// Produce the inspection plan for a candidate.
    ///
    /// Implementors MUST be pure: identical `(context, adapter)`
    /// inputs MUST produce identical outputs. The runtime may
    /// call this more than once per materialisation.
    fn inspection_plan(
        &self,
        context: &CandidateContext<'_>,
    ) -> Result<InspectionPlan, AdapterError>;
}
