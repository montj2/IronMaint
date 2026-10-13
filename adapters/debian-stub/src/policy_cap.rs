//! Debian policy capability (§48, §69).
//!
//! Authority ordering (lowest-precedence first) is the Debian Policy
//! ordering: best-practice and team-policy under formal specification,
//! formal specification and distribution procedure under normative
//! policy. Fedora's stub produces a visibly different order (§80).
//!
//! Obligations: a small, fixed set of mandatory + recommended templates
//! pointing at Debian Policy 4.7.4.1 (source/binary relationship,
//! normative) and Developer's Reference 6.1 (quilt patch series,
//! formal specification).

use std::sync::LazyLock;

use ironmaint_adapter_api::{
    AdapterError, AdapterErrorKind, ObligationTemplate, PolicyCapability, PolicyContext,
    PolicyPlan, verdict_from_evidence_status,
};
use ironmaint_core::AuthorityId;
use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationOutcome, ObligationStrength, PolicyBaseline,
    PolicyReference,
};

static DEBIAN_POLICY: LazyLock<DebianPolicy> = LazyLock::new(DebianPolicy::new);

pub struct DebianPolicy;

impl DebianPolicy {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyCapability for DebianPolicy {
    fn authority_order(&self) -> Vec<AuthorityClassification> {
        vec![
            AuthorityClassification::BestPractice,
            AuthorityClassification::TeamPolicy,
            AuthorityClassification::DistributionProcedure,
            AuthorityClassification::FormalSpecification,
            AuthorityClassification::NormativePolicy,
        ]
    }

    fn derive_obligation_plan(&self, ctx: &PolicyContext) -> Result<PolicyPlan, AdapterError> {
        let baseline = ctx.requested_baseline.cloned().unwrap_or_else(|| {
            PolicyBaseline::new(ctx.candidate.package().package.distribution.clone())
        });

        Ok(PolicyPlan {
            baseline,
            obligation_templates: templates(reference()?, development_reference()?)?,
        })
    }

    /// §48: the verdict is a function of the evidence, not of
    /// what a caller says it is. Refusing a requirement this
    /// adapter never proposed is the other half of that — the
    /// evaluator must not be a general-purpose oracle a caller
    /// can point at an arbitrary question.
    fn evaluate_obligation(
        &self,
        _ctx: &PolicyContext,
        obligation: &ObligationTemplate,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<ObligationOutcome, AdapterError> {
        let known = templates(reference()?, development_reference()?)?;
        if !known
            .iter()
            .any(|t| t.requirement == obligation.requirement)
        {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                format!(
                    "`{}` is not an obligation the Debian stub derives",
                    obligation.requirement
                ),
            ));
        }
        verdict_from_evidence_status(evidence.status)
    }
}

/// Debian Policy 4.7.4.1 — normative.
fn reference() -> Result<PolicyReference, AdapterError> {
    PolicyReference::new(AuthorityId::new())
        .with_section("4.7.4.1")
        .with_title("Source must produce identical binary")
        .with_source("https://www.debian.org/doc/debian-policy/ch-source.html#s-sourcepkgreqs")
        .map_err(|e| AdapterError::new(AdapterErrorKind::InternalAdapterFailure, e.to_string()))
}

/// Debian Developer's Reference 6.1 — formal specification.
fn development_reference() -> Result<PolicyReference, AdapterError> {
    PolicyReference::new(AuthorityId::new())
        .with_section("6.1")
        .with_title("Quilt patch series")
        .with_source(
            "https://www.debian.org/doc/developers-reference/best-pkging-practices.html#quilt",
        )
        .map_err(|e| AdapterError::new(AdapterErrorKind::InternalAdapterFailure, e.to_string()))
}

/// The fixed obligation set this adapter derives.
///
/// One place, so `evaluate_obligation` and `derive_obligation_plan`
/// cannot disagree about what the set is.
fn templates(
    policy_ref: PolicyReference,
    dev_ref: PolicyReference,
) -> Result<Vec<ObligationTemplate>, AdapterError> {
    Ok(vec![
        ObligationTemplate::new(
            policy_ref,
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "binary must be reproducible from source",
        ),
        ObligationTemplate::new(
            dev_ref,
            ObligationStrength::Recommended,
            Applicability::Applicable,
            "debian/patches must use quilt series",
        ),
    ])
}

#[must_use]
pub fn debian_policy() -> &'static DebianPolicy {
    &DEBIAN_POLICY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_order_matches_spec() {
        let p = DebianPolicy::new();
        let order = p.authority_order();
        assert_eq!(
            order,
            vec![
                AuthorityClassification::BestPractice,
                AuthorityClassification::TeamPolicy,
                AuthorityClassification::DistributionProcedure,
                AuthorityClassification::FormalSpecification,
                AuthorityClassification::NormativePolicy,
            ]
        );
    }

    #[test]
    fn derive_plan_includes_two_obligations() {
        let p = DebianPolicy::new();
        let ctx = crate::fixtures::policy_context();
        let plan = p.derive_obligation_plan(&ctx).unwrap();
        assert_eq!(plan.obligation_templates.len(), 2);
        assert_eq!(
            plan.obligation_templates[0].strength,
            ObligationStrength::Mandatory
        );
        assert_eq!(
            plan.obligation_templates[1].strength,
            ObligationStrength::Recommended
        );
    }

    #[test]
    fn derive_plan_baseline_reflects_requested_baseline() {
        let p = DebianPolicy::new();
        let candidate = Box::leak(Box::new(crate::fixtures::source_candidate()));
        let baseline = PolicyBaseline::new(candidate.package().package.distribution.clone())
            .add_authority(AuthorityId::new())
            .add_authority(AuthorityId::new());
        let ctx = PolicyContext {
            package: candidate.package(),
            candidate,
            requested_baseline: Some(&baseline),
        };
        let plan = p.derive_obligation_plan(&ctx).unwrap();
        assert_eq!(plan.baseline.authorities.len(), 2);
    }
}
