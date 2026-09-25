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
    AdapterError, AdapterErrorKind, ObligationTemplate, PolicyCapability, PolicyContext, PolicyPlan,
};
use ironmaint_core::AuthorityId;
use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationStrength, PolicyBaseline, PolicyReference,
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

        let policy_ref = PolicyReference::new(AuthorityId::new())
            .with_section("4.7.4.1")
            .with_title("Source must produce identical binary")
            .with_source("https://www.debian.org/doc/debian-policy/ch-source.html#s-sourcepkgreqs")
            .map_err(|source_err| AdapterError {
                kind: AdapterErrorKind::InternalAdapterFailure,
                message: format!("policy source validation failed: {source_err}"),
            })?;
        let dev_ref = PolicyReference::new(AuthorityId::new())
            .with_section("6.1")
            .with_title("Quilt patch series")
            .with_source(
                "https://www.debian.org/doc/developers-reference/best-pkging-practices.html#quilt",
            )
            .map_err(|source_err| AdapterError {
                kind: AdapterErrorKind::InternalAdapterFailure,
                message: format!("policy source validation failed: {source_err}"),
            })?;

        Ok(PolicyPlan {
            baseline,
            obligation_templates: vec![
                ObligationTemplate {
                    reference: policy_ref,
                    strength: ObligationStrength::Mandatory,
                    applicability: Applicability::Applicable,
                    requirement: "binary must be reproducible from source".to_string(),
                },
                ObligationTemplate {
                    reference: dev_ref,
                    strength: ObligationStrength::Recommended,
                    applicability: Applicability::Applicable,
                    requirement: "debian/patches must use quilt series".to_string(),
                },
            ],
        })
    }
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
