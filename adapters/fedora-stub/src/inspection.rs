//! `InspectionCapability` impl for the Fedora stub (PHASE-1.md §12).
//!
//! The stub advertises [`AdapterCapability::SourceInspection`]
//! (descriptor.rs), so the conformance suite
//! (`ironmaint_testkit::conformance::assert_descriptor_is_valid`)
//! requires this impl to be present. The real Debian adapter's
//! source-intake content lands in 1C.x; the real Fedora adapter
//! (also out of scope for Phase 1) would implement inspection
//! against `fedpkg sources`, `git ls-remote`, and the like.
//!
//! The plan is deliberately hermetic: one `PlannedCheck` with a
//! `SourcePreparation` evidence kind, intended to be filterable
//! by the integration test that asserts "the plan to materialise
//! the inspection check produces the right tuple". The capability
//! key is namespaced under `fedora.inspect.*` per the project's
//! `<adapter-namespace>.<role>.<tool>` convention.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_adapter_api::{
    AdapterError, CandidateContext, InspectionCapability, InspectionPlan, PlannedCheck,
};
use ironmaint_evidence::EvidenceKind;

/// The Fedora stub's `InspectionCapability` impl.
///
/// Returned as a static singleton by [`fedora_inspection`]; the
/// runtime holds a `&'static` reference via `Arc<dyn
/// InspectionCapability>` through the registry, so a singleton is
/// both correct and cheap.
#[derive(Debug, Default, Clone, Copy)]
pub struct FedoraInspection;

impl InspectionCapability for FedoraInspection {
    fn inspection_plan(
        &self,
        _context: &CandidateContext<'_>,
    ) -> Result<InspectionPlan, AdapterError> {
        // The string is a hard-coded constant whose validity the
        // type system cannot prove at compile time (dots, non-empty,
        // ascii). `new` returns a `Result` to surface the
        // failure case; we propagate it rather than panic so the
        // test that pins the constant gets a typed error to assert
        // on instead of a stack trace.
        let key = ToolCapabilityKey::new("fedora.inspect.source_walk")?;
        Ok(InspectionPlan {
            checks: vec![PlannedCheck::new(
                key,
                EvidenceKind::SourcePreparation,
                false,
            )],
        })
    }
}

/// Static accessor. Used by the `DistributionAdapter::inspection`
/// implementation in `lib.rs`.
pub fn fedora_inspection() -> &'static dyn InspectionCapability {
    &FedoraInspection
}
