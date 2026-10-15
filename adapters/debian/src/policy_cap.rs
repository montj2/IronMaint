//! Production `PolicyCapability` impl for the Debian adapter
//! (PHASE-1 §11, §19, §22, §32, §48, §69).
//!
//! 1D.2 lands the real `PolicyDerivation` advertisement on the
//! production adapter. The `PolicyPlan` is built from the
//! `policy/debian/authorities.v1.json` manifest (1D.1):
//!
//! - `baseline` carries every `Authority` id from the manifest,
//!   so the policy baseline is the §19 manifest by construction
//!   and the manifest is the single source of truth.
//! - `obligation_templates` is a single, mandatory,
//!   `Applicability::Applicable` template pinning the
//!   candidate's `debian/control` `Standards-Version` at or
//!   below the manifest's `baseline_policy_version` (currently
//!   `4.7.4.1`).
//!
//! The runtime contract:
//!
//! - `authority_order` returns the 5-class Debian
//!   ordering (lowest-precedence first), matching
//!   `adapters/debian-stub/src/policy_cap.rs:43-51` (the
//!   `§32` "Adapters provide ordering rules" requirement).
//! - `derive_obligation_plan` returns a `PolicyPlan` whose
//!   `obligation_templates` is non-empty (the
//!   `assert_policy_conforms` arm in
//!   `crates/ironmaint-testkit/src/conformance.rs:217-237`
//!   pins this).
//! - `evaluate_obligation` delegates to
//!   `verdict_from_evidence_status` (the §48 default), with
//!   the `InvalidConfiguration` rejection of unrecognised
//!   requirements that
//!   `crates/ironmaint-testkit/src/conformance.rs:340-358`
//!   pins.

use std::sync::LazyLock;

use ironmaint_adapter_api::{
    AdapterError, AdapterErrorKind, ObligationTemplate, PolicyCapability, PolicyContext,
    PolicyPlan, verdict_from_evidence_status,
};
use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationOutcome, ObligationStrength, PolicyBaseline,
    PolicyReference,
};

use crate::authority_manifest::{load_authorities, standards_version_target};

/// The `§48` requirement string this adapter derives.
/// Used as a substring check by `evaluate_obligation` to
/// reject unrecognised obligations.
const STANDARDS_VERSION_REQUIREMENT: &str =
    "debian/control Standards-Version must be at or below the adapter's declared baseline";

/// The §32 classification ordering this adapter
/// produces. Lowest-precedence first. Matches the
/// `adapters/debian-stub/src/policy_cap.rs:43-51`
/// ordering so the conformance suite's per-adapter
/// ordering test holds for both adapters.
const DEBIAN_AUTHORITY_ORDER: &[AuthorityClassification] = &[
    AuthorityClassification::BestPractice,
    AuthorityClassification::TeamPolicy,
    AuthorityClassification::DistributionProcedure,
    AuthorityClassification::FormalSpecification,
    AuthorityClassification::NormativePolicy,
];

/// The production adapter's `PolicyCapability` impl.
///
/// Returned as a static singleton by
/// [`debian_policy_derivation`].
#[derive(Debug, Default, Clone, Copy)]
pub struct DebianPolicyDerivation;

impl DebianPolicyDerivation {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl PolicyCapability for DebianPolicyDerivation {
    fn authority_order(&self) -> Vec<AuthorityClassification> {
        DEBIAN_AUTHORITY_ORDER.to_vec()
    }

    fn derive_obligation_plan(&self, ctx: &PolicyContext) -> Result<PolicyPlan, AdapterError> {
        // The baseline carries the four §19 authorities
        // from the manifest by their `AuthorityId`. The
        // manifest is the single source of truth: a
        // future change to `policy/debian/authorities.v1.json`
        // (a new authority, a re-classification) flows
        // through here without an adapter recompile.
        let authorities = load_authorities();
        let distribution = ctx.candidate.package().package.distribution.clone();
        let mut baseline = ctx
            .requested_baseline
            .cloned()
            .unwrap_or_else(|| PolicyBaseline::new(distribution));
        for authority in authorities {
            baseline = baseline.add_authority(authority.id);
        }

        // One mandatory template, pointing at the
        // `Debian Policy 4.7.4.1` entry's `AuthorityId`.
        // The §48 default evaluator is sufficient: the
        // evidence is produced by the
        // `debian.policy.delta_review` check, whose
        // `verdict` (Pass / Fail) is the §22 gate
        // verdict.
        let debian_policy = authorities
            .iter()
            .find(|a| a.name == "Debian Policy")
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorKind::InternalAdapterFailure,
                    "Debian Policy authority missing from policy/debian/authorities.v1.json",
                )
            })?;

        let policy_ref = PolicyReference::new(debian_policy.id)
            .with_section("4.7.4.1")
            .with_title("Source must declare current Standards-Version")
            .with_source("https://www.debian.org/doc/debian-policy/ch-source.html#s-sourcepkgreqs")
            .map_err(|e| {
                AdapterError::new(AdapterErrorKind::InternalAdapterFailure, e.to_string())
            })?;

        let template = ObligationTemplate::new(
            policy_ref,
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            format!(
                "{STANDARDS_VERSION_REQUIREMENT} ({})",
                standards_version_target()
            ),
        );

        Ok(PolicyPlan {
            baseline,
            obligation_templates: vec![template],
        })
    }

    fn evaluate_obligation(
        &self,
        _ctx: &PolicyContext,
        obligation: &ObligationTemplate,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<ObligationOutcome, AdapterError> {
        // §48: the evaluator is a function of the
        // named policy statement, not a
        // general-purpose oracle. Refusing a
        // requirement this adapter never proposed is
        // the conformance suite's
        // `assert_obligation_evaluation_conforms` arm
        // (lines 340-358).
        if !obligation
            .requirement
            .starts_with(STANDARDS_VERSION_REQUIREMENT)
        {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                format!(
                    "`{}` is not an obligation the Debian production adapter derives",
                    obligation.requirement
                ),
            ));
        }
        verdict_from_evidence_status(evidence.status)
    }
}

/// Static accessor. Used by the
/// `DistributionAdapter::policy` implementation in `lib.rs`.
#[must_use]
pub fn debian_policy_derivation() -> &'static dyn PolicyCapability {
    static INSTANCE: LazyLock<DebianPolicyDerivation> = LazyLock::new(DebianPolicyDerivation::new);
    &*INSTANCE
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{
        CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
        GitHashAlgorithm, GitObjectId, JobId, PackageIdentity, PackageName, PackageRevision,
        PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
    };
    use ironmaint_evidence::{Evidence, EvidenceKind, EvidenceProducer, EvidenceScope};
    use time::macros::datetime;
    use url::Url;

    fn hex(c: char, n: usize) -> String {
        std::iter::repeat_n(c, n).collect()
    }

    fn candidate() -> Box<SourceCandidate> {
        let repo = RepositoryRef::new(
            VcsKind::Git,
            Url::parse("https://example.invalid/foo.git").unwrap(),
        )
        .unwrap();
        Box::new(SourceCandidate::new(
            JobId::new(),
            PackageRevision::new(
                PackageIdentity::new(
                    DistributionRef::new(
                        DistributionFamily::new("debian").unwrap(),
                        DistributionRelease::new("sid").unwrap(),
                    ),
                    PackageName::new("example").unwrap(),
                ),
                PackageVersion::new("1.0.0").unwrap(),
            ),
            repo,
            GitObjectId::new(GitHashAlgorithm::Sha1, hex('a', 40)).unwrap(),
            GitObjectId::new(GitHashAlgorithm::Sha1, hex('b', 40)).unwrap(),
            datetime!(2026-01-01 00:00:00 UTC),
        ))
    }

    /// A minimal `Evidence` for the unit tests that
    /// don't care about the evidence's other
    /// fields. The fingerprint, scope, and producer
    /// are pinned to representative defaults so the
    /// test fixtures are stable.
    fn make_evidence(status: ironmaint_evidence::EvidenceStatus) -> Evidence {
        // 64-char hex BLAKE3 digest (CandidateFingerprint's
        // structural requirement).
        let fp_hex = "0".repeat(64);
        Evidence::new(
            CandidateFingerprint::from_hex(fp_hex).expect("static 64-char hex"),
            EvidenceKind::PolicyEvaluation,
            status,
            EvidenceProducer::new("test"),
            EvidenceScope::Job(JobId::new()),
            datetime!(2026-01-01 00:00:00 UTC),
        )
    }

    /// The §32 ordering is the 5-class Debian
    /// ordering, lowest-precedence first. The stub
    /// has the same ordering so the cross-adapter
    /// conformance arm holds.
    #[test]
    fn authority_order_matches_spec() {
        let p = DebianPolicyDerivation::new();
        assert_eq!(
            p.authority_order(),
            vec![
                AuthorityClassification::BestPractice,
                AuthorityClassification::TeamPolicy,
                AuthorityClassification::DistributionProcedure,
                AuthorityClassification::FormalSpecification,
                AuthorityClassification::NormativePolicy,
            ]
        );
    }

    /// The plan's baseline carries the four §19
    /// authorities from the manifest by their
    /// `AuthorityId`. The manifest is the single
    /// source of truth; the plan reflects it
    /// structurally.
    #[test]
    fn derive_plan_baseline_reflects_the_manifest() {
        let p = DebianPolicyDerivation::new();
        let candidate = candidate();
        let ctx = PolicyContext {
            package: candidate.package(),
            candidate: &candidate,
            requested_baseline: None,
        };
        let plan = p.derive_obligation_plan(&ctx).unwrap();
        let expected_count = load_authorities().len();
        assert_eq!(plan.baseline.authorities.len(), expected_count);
    }

    /// The plan derives one mandatory,
    /// `Applicability::Applicable` template pinning
    /// the `Standards-Version` at or below the
    /// manifest's `baseline_policy_version`.
    #[test]
    fn derive_plan_includes_one_standards_version_obligation() {
        let p = DebianPolicyDerivation::new();
        let candidate = candidate();
        let ctx = PolicyContext {
            package: candidate.package(),
            candidate: &candidate,
            requested_baseline: None,
        };
        let plan = p.derive_obligation_plan(&ctx).unwrap();
        assert_eq!(plan.obligation_templates.len(), 1);
        let template = &plan.obligation_templates[0];
        assert_eq!(template.strength, ObligationStrength::Mandatory);
        assert_eq!(template.applicability, Applicability::Applicable);
        assert!(
            template
                .requirement
                .starts_with(STANDARDS_VERSION_REQUIREMENT),
            "the requirement names the §22 standards-version gate: {}",
            template.requirement
        );
        // The plan's reference points at the manifest's
        // `Debian Policy` entry by id.
        let debian_policy_id = load_authorities()
            .iter()
            .find(|a| a.name == "Debian Policy")
            .map(|a| a.id)
            .expect("Debian Policy authority is a manifest entry");
        assert_eq!(template.reference.authority, debian_policy_id);
    }

    /// The plan's baseline reflects a requested
    /// baseline (caller-supplied authorities appear
    /// before the manifest's four). The `PolicyPlan`
    /// is additive over a `requested_baseline`.
    #[test]
    fn derive_plan_baseline_combines_with_a_requested_baseline() {
        use ironmaint_core::AuthorityId;

        let p = DebianPolicyDerivation::new();
        let candidate = candidate();
        let mut requested = PolicyBaseline::new(candidate.package().package.distribution.clone());
        requested = requested.add_authority(AuthorityId::new());
        let ctx = PolicyContext {
            package: candidate.package(),
            candidate: &candidate,
            requested_baseline: Some(&requested),
        };
        let plan = p.derive_obligation_plan(&ctx).unwrap();
        assert_eq!(
            plan.baseline.authorities.len(),
            1 + load_authorities().len(),
            "the requested baseline's one authority plus the manifest's four"
        );
    }

    /// `evaluate_obligation` is the §48 default
    /// mapping. The conformance suite walks every
    /// (template, status) pair and asserts that
    /// `Fail`-status evidence never reads as `Pass`.
    #[test]
    fn evaluate_obligation_delegates_to_the_default_verdict() {
        use ironmaint_evidence::EvidenceStatus as S;

        let p = DebianPolicyDerivation::new();
        let candidate = candidate();
        let ctx = PolicyContext {
            package: candidate.package(),
            candidate: &candidate,
            requested_baseline: None,
        };
        let plan = p.derive_obligation_plan(&ctx).unwrap();
        let template = &plan.obligation_templates[0];
        for status in [S::Pass, S::Fail, S::NotApplicable] {
            let evidence = make_evidence(status);
            let verdict = p
                .evaluate_obligation(&ctx, template, &evidence)
                .expect("the default verdict resolves");
            match status {
                S::Pass => assert_eq!(verdict, ObligationOutcome::Pass),
                S::Fail => assert_eq!(verdict, ObligationOutcome::Fail),
                S::NotApplicable => assert_eq!(verdict, ObligationOutcome::Pass),
                _ => unreachable!(),
            }
        }
    }

    /// A requirement this adapter never derived is
    /// rejected with `InvalidConfiguration`. The
    /// conformance suite's
    /// `assert_obligation_evaluation_conforms` arm
    /// (lines 340-358) pins this.
    #[test]
    fn evaluate_obligation_rejects_unrecognised_requirements() {
        use ironmaint_core::AuthorityId;
        use ironmaint_evidence::EvidenceStatus as S;
        use ironmaint_policy::PolicyReference;

        let p = DebianPolicyDerivation::new();
        let candidate = candidate();
        let ctx = PolicyContext {
            package: candidate.package(),
            candidate: &candidate,
            requested_baseline: None,
        };
        let bogus = ObligationTemplate::new(
            PolicyReference::new(AuthorityId::new()),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "unrecognised requirement string",
        );
        let evidence = make_evidence(S::Pass);
        let err = p
            .evaluate_obligation(&ctx, &bogus, &evidence)
            .expect_err("an unrecognised requirement is a configuration error");
        assert_eq!(err.kind, AdapterErrorKind::InvalidConfiguration);
    }
}
