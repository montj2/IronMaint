//! Policy capability (§48).
//!
//! Adapters describe how their distribution orders authority
//! classifications and propose obligation templates for a given
//! candidate context. The state engine decides which of these
//! templates become [`ironmaint_policy::Obligation`]s and whether
//! they block a transition.

use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationStrength, PolicyBaseline, PolicyReference,
};

use crate::contexts::PolicyContext;
use crate::error::AdapterError;

/// A pre-candidate obligation template.
///
/// Distinct from [`ironmaint_policy::Obligation`]: no id, no candidate
/// binding, no status, no evidence. Pure shape description that the
/// executor (Phase 0B) materializes into concrete [`ironmaint_policy::Obligation`]
/// values bound to a fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObligationTemplate {
    pub reference: PolicyReference,
    pub strength: ObligationStrength,
    pub applicability: Applicability,
    pub requirement: String,
}

impl ObligationTemplate {
    #[must_use]
    pub fn new(
        reference: PolicyReference,
        strength: ObligationStrength,
        applicability: Applicability,
        requirement: impl Into<String>,
    ) -> Self {
        Self {
            reference,
            strength,
            applicability,
            requirement: requirement.into(),
        }
    }
}

/// The bundle an adapter produces for [`PolicyCapability::derive_obligation_plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyPlan {
    pub baseline: PolicyBaseline,
    pub obligation_templates: Vec<ObligationTemplate>,
}

impl PolicyPlan {
    #[must_use]
    pub fn new(baseline: PolicyBaseline) -> Self {
        Self {
            baseline,
            obligation_templates: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_template(mut self, template: ObligationTemplate) -> Self {
        self.obligation_templates.push(template);
        self
    }
}

/// Optional capability: derive a policy plan for a candidate (§48).
///
/// Not every adapter has policy authority (a metadata-only adapter
/// may not), so the trait is returned as `Option<&dyn …>` from
/// [`crate::DistributionAdapter::policy`].
pub trait PolicyCapability: Send + Sync + 'static {
    /// Ordered list of authority classifications, lowest-precedence
    /// first (§32 "Adapters provide ordering rules").
    fn authority_order(&self) -> Vec<AuthorityClassification>;

    /// Derive the policy plan for one candidate.
    ///
    /// # Errors
    /// Returns `AdapterError { kind: PolicyUnavailable, .. }` when
    /// the adapter cannot retrieve authority data.
    fn derive_obligation_plan(&self, context: &PolicyContext) -> Result<PolicyPlan, AdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{AuthorityId, DistributionFamily, DistributionRef, DistributionRelease};
    use ironmaint_policy::{PolicyBaseline, PolicyReference};

    #[test]
    fn policy_plan_builder_chain() {
        let baseline = PolicyBaseline::new(DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("sid").unwrap(),
        ))
        .add_authority(AuthorityId::new());

        let t = ObligationTemplate::new(
            PolicyReference::new(AuthorityId::new()),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "must build reproducibly",
        );
        let plan = PolicyPlan::new(baseline.clone()).with_template(t);
        assert_eq!(plan.obligation_templates.len(), 1);
        assert_eq!(plan.baseline.distribution, baseline.distribution);
    }

    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn PolicyCapability>();
    }
}
