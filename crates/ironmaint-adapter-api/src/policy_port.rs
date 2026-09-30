//! Policy capability (§48).
//!
//! Adapters describe how their distribution orders authority
//! classifications and propose obligation templates for a given
//! candidate context. The state engine decides which of these
//! templates become [`ironmaint_policy::Obligation`]s and whether
//! they block a transition.

use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationOutcome, ObligationStrength, PolicyBaseline,
    PolicyReference,
};

use crate::contexts::PolicyContext;
use crate::error::{AdapterError, AdapterErrorKind};

/// The §48 default verdict: an obligation is satisfied, or not, by
/// the evidence a policy-evaluation check produced — and by nothing
/// else.
///
/// Two statuses have no verdict here, and that is the interesting
/// part of the mapping rather than an omission:
///
/// - `InfrastructureError` means the evaluator did not run. Folding
///   it into `Fail` would make a broken sandbox indistinguishable
///   from a policy violation, and the remedy for the two is
///   opposite: one is an operator problem, the other is a repair
///   the agent should attempt.
/// - `Inconclusive` means the evaluator ran and declined to answer,
///   which is `RequiresReview` by definition — a person weighs in,
///   the runtime does not guess either way.
///
/// # Errors
///
/// Returns `AdapterError { kind: PolicyUnavailable, .. }` for
/// `InfrastructureError` and
/// `AdapterError { kind: HumanReviewRequired, .. }` for
/// `Inconclusive`. The caller decides what to do with a missing
/// verdict; this function does not invent one.
pub fn verdict_from_evidence_status(
    status: ironmaint_evidence::EvidenceStatus,
) -> Result<ObligationOutcome, AdapterError> {
    use ironmaint_evidence::EvidenceStatus as S;
    Ok(match status {
        S::Pass => ObligationOutcome::Pass,
        S::Fail => ObligationOutcome::Fail,
        S::NotApplicable => ObligationOutcome::Pass,
        S::Inconclusive => {
            return Err(AdapterError::new(
                AdapterErrorKind::HumanReviewRequired,
                "the policy evaluator was inconclusive; a person must weigh in",
            ));
        }
        S::InfrastructureError => {
            return Err(AdapterError::new(
                AdapterErrorKind::PolicyUnavailable,
                "the policy evaluator did not run; there is no verdict to record",
            ));
        }
    })
}

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

    /// The verdict a deterministic policy evaluator reaches for
    /// one derived obligation, given the evidence a
    /// policy-evaluation check produced (§48).
    ///
    /// §48: "A deterministic fixture policy evaluator produces the
    /// evidence. This is not an LLM judgment." So the verdict is a
    /// function of the requirement and the evidence — never of
    /// what a caller says the verdict is.
    ///
    /// This is a port method rather than a command, and there is
    /// deliberately no MCP tool for it. §53 lists
    /// `ironmaint_obligation_set_pass` among the tools that must
    /// **not** be implemented, and §102 item 25 says the same: MCP
    /// cannot set obligations. An obligation's verdict is a fact
    /// about an evaluation that ran, so the runtime records it
    /// when the evidence lands.
    ///
    /// # Errors
    /// Returns `AdapterError { kind: PolicyUnavailable, .. }` when
    /// the evaluator cannot be reached, and
    /// `AdapterError { kind: InvalidConfiguration, .. }` when
    /// `obligation` is not one this adapter derived — a caller
    /// must not be able to have the adapter rule on a
    /// requirement it never proposed.
    fn evaluate_obligation(
        &self,
        context: &PolicyContext,
        obligation: &ObligationTemplate,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<ObligationOutcome, AdapterError>;
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

    use ironmaint_evidence::EvidenceStatus as S;

    #[test]
    fn the_default_verdict_follows_the_evidence_status() {
        assert_eq!(
            verdict_from_evidence_status(S::Pass).unwrap(),
            ObligationOutcome::Pass
        );
        assert_eq!(
            verdict_from_evidence_status(S::Fail).unwrap(),
            ObligationOutcome::Fail
        );
        // A check that did not apply is not a policy violation, so
        // it is not a failure either — but it is emphatically not a
        // claim that the policy was evaluated and found clean.
        assert_eq!(
            verdict_from_evidence_status(S::NotApplicable).unwrap(),
            ObligationOutcome::Pass
        );
    }

    /// The two arms that produce no verdict. They are the ones the
    /// conformance suite deliberately does *not* pin — it only
    /// requires that failing evidence never read as `Pass`, so a
    /// real adapter is free to be stricter than the default — which
    /// leaves these two the default's own responsibility.
    #[test]
    fn a_declining_evaluator_produces_no_verdict() {
        let inconclusive = verdict_from_evidence_status(S::Inconclusive).unwrap_err();
        assert_eq!(
            inconclusive.kind,
            AdapterErrorKind::HumanReviewRequired,
            "an evaluator that ran and declined is a question for a person"
        );

        let broken = verdict_from_evidence_status(S::InfrastructureError).unwrap_err();
        assert_eq!(
            broken.kind,
            AdapterErrorKind::PolicyUnavailable,
            "an evaluator that never ran is an operator problem, not a policy verdict"
        );
    }
}
