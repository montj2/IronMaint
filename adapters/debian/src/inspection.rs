//! Production `InspectionCapability` impl for the Debian adapter
//! (PHASE-1 §12, §13).
//!
//! 1B.3 + 1C.1 + 1C.2 ship two `PlannedCheck`s with the
//! forward-pointer keys `debian.inspect.source_preparation` (the
//! 1C.1 six-check summary) and `debian.inspect.source_analysis`
//! (the 1C.2 comprehensive source-tree inventory per §18). The
//! actual executor-side tool registrations for these keys are
//! the 1C.x work; this module only needs the *plan* shape so
//! the conformance arm at `assert_descriptor_is_valid` for
//! `AdapterCapability::SourceInspection` passes.
//!
//! The plan is hermetic: identical `(context, adapter)` inputs
//! produce identical outputs, no I/O, no side effects, no
//! distribution of its own (§12). `PlannedCheck::mandatory` is
//! `true` for every check because every captured Debian
//! candidate needs the source-preparation and source-analysis
//! checks; the runtime's
//! `gate_stage_for(EvidenceKind::SourcePreparation)` and
//! `gate_stage_for(EvidenceKind::SourceIntegrity)` mappings
//! (in `ironmaint-state`) route each check to the right gate.

use ironmaint_adapter_api::{
    AdapterError, CandidateContext, InspectionCapability, InspectionPlan, PlannedCheck,
    ToolCapabilityKey,
};
use ironmaint_evidence::EvidenceKind;

/// The production Debian adapter's `InspectionCapability` impl.
///
/// Returned as a static singleton by [`debian_inspection`].
#[derive(Debug, Default, Clone, Copy)]
pub struct DebianInspection;

impl InspectionCapability for DebianInspection {
    fn inspection_plan(
        &self,
        _context: &CandidateContext<'_>,
    ) -> Result<InspectionPlan, AdapterError> {
        // The strings are hard-coded constants whose validity
        // the type system cannot prove at compile time (dots,
        // non-empty, ascii). `new` returns a `Result` to
        // surface the failure case; we propagate it rather
        // than panic so the test that pins the constant gets
        // a typed error to assert on instead of a stack trace.
        let source_preparation = ToolCapabilityKey::new("debian.inspect.source_preparation")?;
        let source_analysis = ToolCapabilityKey::new("debian.inspect.source_analysis")?;
        Ok(InspectionPlan {
            checks: vec![
                PlannedCheck::new(source_preparation, EvidenceKind::SourcePreparation, true),
                PlannedCheck::new(source_analysis, EvidenceKind::SourceIntegrity, true),
            ],
        })
    }
}

/// Static accessor. Used by the `DistributionAdapter::inspection`
/// implementation in `lib.rs`.
#[must_use]
pub fn debian_inspection() -> &'static dyn InspectionCapability {
    &DebianInspection
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{
        DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId,
        JobId, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
        SourceCandidate, VcsKind,
    };
    use time::macros::datetime;
    use url::Url;

    fn hex(c: char, n: usize) -> String {
        std::iter::repeat_n(c, n).collect()
    }

    fn candidate() -> SourceCandidate {
        let repo = RepositoryRef::new(
            VcsKind::Git,
            Url::parse("https://example.invalid/foo.git").unwrap(),
        )
        .unwrap();
        SourceCandidate::new(
            JobId::new(),
            PackageRevision::new(
                PackageIdentity::new(
                    DistributionRef::new(
                        DistributionFamily::new("debian").unwrap(),
                        DistributionRelease::new("sid").unwrap(),
                    ),
                    PackageName::new("foo").unwrap(),
                ),
                PackageVersion::new("1.0.0").unwrap(),
            ),
            repo,
            GitObjectId::new(GitHashAlgorithm::Sha1, hex('a', 40)).unwrap(),
            GitObjectId::new(GitHashAlgorithm::Sha1, hex('b', 40)).unwrap(),
            datetime!(2026-01-01 00:00:00 UTC),
        )
    }

    /// The 1C.1 + 1C.2 plan shape: two checks, with the
    /// 1C.1 source-preparation key and the 1C.2
    /// source-analysis key, mapped to the corresponding
    /// `EvidenceKind` values.
    #[test]
    fn hermetic_plan_has_two_checks_for_source_preparation_and_source_analysis() {
        let i = DebianInspection;
        let c = candidate();
        let ctx = CandidateContext {
            package: c.package(),
            candidate: &c,
        };
        let plan = i.inspection_plan(&ctx).unwrap();
        assert_eq!(plan.checks.len(), 2);
        assert_eq!(
            plan.checks[0].key.as_str(),
            "debian.inspect.source_preparation"
        );
        assert_eq!(
            plan.checks[0].evidence_kind,
            EvidenceKind::SourcePreparation
        );
        assert!(plan.checks[0].mandatory);
        assert_eq!(
            plan.checks[1].key.as_str(),
            "debian.inspect.source_analysis"
        );
        assert_eq!(plan.checks[1].evidence_kind, EvidenceKind::SourceIntegrity);
        assert!(plan.checks[1].mandatory);
    }
}
