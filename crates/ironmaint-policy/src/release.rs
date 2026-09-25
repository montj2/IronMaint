//! Release coordination types — release candidates and publication plans (§§42–43).
//!
//! [`ReleaseCandidate`] is the durable "snapshot of everything required
//! to release": the source fingerprint, the policy baseline, and the
//! collections of obligation, gate, and issue-action IDs that the
//! state machine consumed. Creation of a `ReleaseCandidate` does NOT
//! imply releasability (§42); the state engine is the only authority
//! on whether the candidate reaches `ReadyForApproval` (§20).
//!
//! [`PublicationPlan`] pairs a `ReleaseCandidate` with the
//! distribution it's intended for and the [`PrivilegedOperation`]s
//! required to ship. Operations remain in
//! [`AuthorizationState::Proposed`] here; the privileged service
//! transitions them forward.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::{
    CandidateFingerprint, DistributionRef, GateId, IssueActionId, JobId, ObligationId,
    ReleaseCandidateId, SchemaVersion,
};

use crate::{PolicyBaseline, PrivilegedOperation};

/// A snapshot of "everything required to release" for one candidate (§42).
///
/// `policy_baseline` carries the [`AuthorityId`]s the candidate is
/// held against. The candidate is releasable only after the state
/// machine's `FinalValidation → ReadyForApproval → Approved →
/// PublicationPending → Published` path completes (and only when all
/// referenced obligations / gates / issue actions have cleared).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReleaseCandidate {
    pub id: ReleaseCandidateId,
    pub job_id: JobId,
    pub source: CandidateFingerprint,
    pub policy_baseline: PolicyBaseline,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obligation_ids: Vec<ObligationId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_ids: Vec<GateId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issue_actions: Vec<IssueActionId>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Wire-format schema version (§60). Always serializes as
    /// `SchemaVersion::V1`; on parse, a missing field defaults to `V1`.
    #[serde(default)]
    pub schema_version: SchemaVersion,
}

impl ReleaseCandidate {
    #[must_use]
    pub fn new(
        job_id: JobId,
        source: CandidateFingerprint,
        policy_baseline: PolicyBaseline,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            id: ReleaseCandidateId::new(),
            job_id,
            source,
            policy_baseline,
            obligation_ids: Vec::new(),
            gate_ids: Vec::new(),
            issue_actions: Vec::new(),
            created_at,
            schema_version: SchemaVersion::default(),
        }
    }

    #[must_use]
    pub fn with_obligation(mut self, id: ObligationId) -> Self {
        self.obligation_ids.push(id);
        self
    }

    #[must_use]
    pub fn with_gate(mut self, id: GateId) -> Self {
        self.gate_ids.push(id);
        self
    }

    #[must_use]
    pub fn with_issue_action(mut self, id: IssueActionId) -> Self {
        self.issue_actions.push(id);
        self
    }
}

/// The pre-execution bundle of privileged operations required to
/// publish a release (§43).
///
/// `operations` carry the side-effects required to publish; whether
/// each is `Authorized` / `Executing` / `Succeeded` belongs to the
/// privileged service and the executor (Phase 0B). IronMaint model
/// objects only describe what must happen.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicationPlan {
    pub release_candidate: ReleaseCandidateId,
    pub distribution: DistributionRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<PrivilegedOperation>,
    /// Wire-format schema version (§60). Always serializes as
    /// `SchemaVersion::V1`; on parse, a missing field defaults to `V1`.
    #[serde(default)]
    pub schema_version: SchemaVersion,
}

impl PublicationPlan {
    #[must_use]
    pub fn new(release_candidate: ReleaseCandidateId, distribution: DistributionRef) -> Self {
        Self {
            release_candidate,
            distribution,
            operations: Vec::new(),
            schema_version: SchemaVersion::default(),
        }
    }

    #[must_use]
    pub fn with_operation(mut self, op: PrivilegedOperation) -> Self {
        self.operations.push(op);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{
        AuthorityId, CandidateFingerprint, DistributionFamily, DistributionRelease,
    };
    use time::macros::datetime;

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    fn dref() -> DistributionRef {
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("sid").unwrap(),
        )
    }

    fn baseline() -> PolicyBaseline {
        PolicyBaseline::new(dref()).add_authority(AuthorityId::new())
    }

    #[test]
    fn release_candidate_starts_empty() {
        let rc = ReleaseCandidate::new(
            JobId::new(),
            fp(),
            baseline(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        assert!(rc.obligation_ids.is_empty());
        assert!(rc.gate_ids.is_empty());
        assert!(rc.issue_actions.is_empty());
    }

    #[test]
    fn release_candidate_builder_accumulates() {
        let rc = ReleaseCandidate::new(
            JobId::new(),
            fp(),
            baseline(),
            datetime!(2026-01-01 00:00:00 UTC),
        )
        .with_obligation(ObligationId::new())
        .with_obligation(ObligationId::new())
        .with_gate(GateId::new())
        .with_issue_action(IssueActionId::new());
        assert_eq!(rc.obligation_ids.len(), 2);
        assert_eq!(rc.gate_ids.len(), 1);
        assert_eq!(rc.issue_actions.len(), 1);
    }

    #[test]
    fn release_candidate_round_trips() {
        let rc = ReleaseCandidate::new(
            JobId::new(),
            fp(),
            baseline(),
            datetime!(2026-02-02 12:00:00 UTC),
        )
        .with_obligation(ObligationId::new());
        let json = serde_json::to_string(&rc).unwrap();
        let parsed: ReleaseCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rc);
    }

    #[test]
    fn publication_plan_starts_empty() {
        let plan = PublicationPlan::new(ReleaseCandidateId::new(), dref());
        assert!(plan.operations.is_empty());
    }

    #[test]
    fn publication_plan_round_trips() {
        let op = PrivilegedOperation::proposed(
            crate::PrivilegedOperationKind::CanonicalRepositoryPush,
            fp(),
        );
        let plan = PublicationPlan::new(ReleaseCandidateId::new(), dref()).with_operation(op);
        let json = serde_json::to_string(&plan).unwrap();
        let parsed: PublicationPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, plan);
    }
}
