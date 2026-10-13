//! Fedora policy capability (§48, §69).
//!
//! Authority ordering differs from Debian (§80): Fedora adds
//! `LocalPolicy` between `DistributionProcedure` and `FormalSpecification`,
//! reflecting the FPC (Fedora Packaging Committee) decision-snapshot
//! policy. The base ordering otherwise mirrors Debian's.
//!
//! Obligations: Fedora Packaging Guidelines (Normative) + an
//! FPC decision snapshot (LocalPolicy) for the .fc tag policy.

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

static FEDORA_POLICY: LazyLock<FedoraPolicy> = LazyLock::new(FedoraPolicy::new);

pub struct FedoraPolicy;

impl FedoraPolicy {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FedoraPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyCapability for FedoraPolicy {
    fn authority_order(&self) -> Vec<AuthorityClassification> {
        vec![
            AuthorityClassification::BestPractice,
            AuthorityClassification::TeamPolicy,
            AuthorityClassification::DistributionProcedure,
            AuthorityClassification::LocalPolicy,
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
            obligation_templates: templates(packaging_reference()?, fpc_reference()?)?,
        })
    }

    /// §48: the verdict is a function of the evidence, not of what
    /// a caller says it is. Refusing a requirement this adapter
    /// never proposed is the other half of that — the evaluator
    /// must not be a general-purpose oracle a caller can point at
    /// an arbitrary question.
    fn evaluate_obligation(
        &self,
        _ctx: &PolicyContext,
        obligation: &ObligationTemplate,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<ObligationOutcome, AdapterError> {
        let known = templates(packaging_reference()?, fpc_reference()?)?;
        if !known
            .iter()
            .any(|t| t.requirement == obligation.requirement)
        {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                format!(
                    "`{}` is not an obligation the Fedora stub derives",
                    obligation.requirement
                ),
            ));
        }
        verdict_from_evidence_status(evidence.status)
    }
}

/// Fedora Packaging Guidelines — normative.
fn packaging_reference() -> Result<PolicyReference, AdapterError> {
    PolicyReference::new(AuthorityId::new())
        .with_section("Packaging Guidelines")
        .with_title("License must be OSI-approved")
        .with_source("https://docs.fedoraproject.org/en-US/packaging-guidelines/")
        .map_err(|e| AdapterError::new(AdapterErrorKind::InternalAdapterFailure, e.to_string()))
}

/// FPC decision snapshot — local policy, the authority class
/// Debian's ordering does not have (§80).
fn fpc_reference() -> Result<PolicyReference, AdapterError> {
    PolicyReference::new(AuthorityId::new())
        .with_section("FPC Decision 2018-01")
        .with_title("Dist tag policy (.fcNN)")
        .with_source("https://fedoraproject.org/wiki/Packaging:DistTag")
        .map_err(|e| AdapterError::new(AdapterErrorKind::InternalAdapterFailure, e.to_string()))
}

/// The fixed obligation set this adapter derives.
///
/// One place, so `evaluate_obligation` and `derive_obligation_plan`
/// cannot disagree about what the set is.
fn templates(
    packaging_ref: PolicyReference,
    fpc_ref: PolicyReference,
) -> Result<Vec<ObligationTemplate>, AdapterError> {
    Ok(vec![
        ObligationTemplate::new(
            packaging_ref,
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "license must be OSI-approved",
        ),
        ObligationTemplate::new(
            fpc_ref,
            ObligationStrength::LocalPolicy,
            Applicability::Applicable,
            "release field must end with .fcNN tag",
        ),
    ])
}

#[must_use]
pub fn fedora_policy() -> &'static FedoraPolicy {
    &FEDORA_POLICY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_order_inserts_local_policy_between_procedure_and_formal_spec() {
        // §80: Fedora's authority order differs from Debian by
        // inserting LocalPolicy after DistributionProcedure.
        let p = FedoraPolicy::new();
        let order = p.authority_order();
        assert_eq!(
            order,
            vec![
                AuthorityClassification::BestPractice,
                AuthorityClassification::TeamPolicy,
                AuthorityClassification::DistributionProcedure,
                AuthorityClassification::LocalPolicy,
                AuthorityClassification::FormalSpecification,
                AuthorityClassification::NormativePolicy,
            ]
        );
    }

    #[test]
    fn derive_plan_includes_two_obligations() {
        let p = FedoraPolicy::new();
        let ctx = crate::fixtures::policy_context();
        let plan = p.derive_obligation_plan(&ctx).unwrap();
        assert_eq!(plan.obligation_templates.len(), 2);
        assert_eq!(
            plan.obligation_templates[0].strength,
            ObligationStrength::Mandatory
        );
        assert_eq!(
            plan.obligation_templates[1].strength,
            ObligationStrength::LocalPolicy
        );
    }
}
