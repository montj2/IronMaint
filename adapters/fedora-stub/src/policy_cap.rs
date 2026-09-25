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
    AdapterError, AdapterErrorKind, ObligationTemplate, PolicyCapability, PolicyContext, PolicyPlan,
};
use ironmaint_core::AuthorityId;
use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationStrength, PolicyBaseline, PolicyReference,
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

        let packaging_ref = PolicyReference::new(AuthorityId::new())
            .with_section("Packaging Guidelines")
            .with_title("License must be OSI-approved")
            .with_source("https://docs.fedoraproject.org/en-US/packaging-guidelines/")
            .map_err(|source_err| AdapterError {
                kind: AdapterErrorKind::InternalAdapterFailure,
                message: format!("policy source validation failed: {source_err}"),
            })?;
        let fpc_ref = PolicyReference::new(AuthorityId::new())
            .with_section("FPC Decision 2018-01")
            .with_title("Dist tag policy (.fcNN)")
            .with_source("https://fedoraproject.org/wiki/Packaging:DistTag")
            .map_err(|source_err| AdapterError {
                kind: AdapterErrorKind::InternalAdapterFailure,
                message: format!("policy source validation failed: {source_err}"),
            })?;

        Ok(PolicyPlan {
            baseline,
            obligation_templates: vec![
                ObligationTemplate {
                    reference: packaging_ref,
                    strength: ObligationStrength::Mandatory,
                    applicability: Applicability::Applicable,
                    requirement: "license must be OSI-approved".to_string(),
                },
                ObligationTemplate {
                    reference: fpc_ref,
                    strength: ObligationStrength::LocalPolicy,
                    applicability: Applicability::Applicable,
                    requirement: "release field must end with .fcNN tag".to_string(),
                },
            ],
        })
    }
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
