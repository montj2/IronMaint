//! Maintenance job identity, state, and projection (§§15–17).
//!
//! The state machine itself (`TransitionEngine`, transition rules,
//! blockers) is in `ironmaint-state` (lands in 0A.3). This module
//! holds only the value types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::identity::MaintenanceEventId;
use crate::identity::{CandidateId, JobId};
use crate::package::PackageIdentity;

/// Workflow state for a [`MaintenanceJob`].
///
/// PHASE-0A.md §16 lists the canonical 18-state workflow, but
/// `CandidateAssembly` appears twice — likely a spec typo. We
/// replace the **second** occurrence with `SourceRevision`,
/// producing 19 distinct variants:
///
/// ```text
/// EventDetected, Intake, SourceReview, CandidateAssembly, SourceRevision,
/// SourceIntegrity, BuildValidation, PackageQaValidation,
/// FunctionalValidation, UpgradeValidation, ReleaseReview, FinalValidation,
/// ReadyForApproval, Approved, PublicationPending, Published,
/// HumanReviewRequired, InfrastructureBlocked, Cancelled
/// ```
///
/// `SourceRevision` is a manual source-tweak window between the
/// initial candidate build and the source-integrity check; an
/// operator may edit packaging files there without invalidating the
/// state machine. The state machine in 0A.3 will encode the rules
/// for entering and leaving this state.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    EventDetected,
    Intake,
    SourceReview,
    CandidateAssembly,
    SourceRevision,
    SourceIntegrity,
    BuildValidation,
    PackageQaValidation,
    FunctionalValidation,
    UpgradeValidation,
    ReleaseReview,
    FinalValidation,
    ReadyForApproval,
    Approved,
    PublicationPending,
    Published,
    HumanReviewRequired,
    InfrastructureBlocked,
    Cancelled,
}

impl JobState {
    /// All variants, in declaration order.
    ///
    /// Used by tests to assert that no variant is silently dropped
    /// during maintenance.
    pub const ALL: [Self; 19] = [
        Self::EventDetected,
        Self::Intake,
        Self::SourceReview,
        Self::CandidateAssembly,
        Self::SourceRevision,
        Self::SourceIntegrity,
        Self::BuildValidation,
        Self::PackageQaValidation,
        Self::FunctionalValidation,
        Self::UpgradeValidation,
        Self::ReleaseReview,
        Self::FinalValidation,
        Self::ReadyForApproval,
        Self::Approved,
        Self::PublicationPending,
        Self::Published,
        Self::HumanReviewRequired,
        Self::InfrastructureBlocked,
        Self::Cancelled,
    ];

    /// Terminal states — no further transitions are possible.
    ///
    /// `Published` is the success terminal; `Cancelled` is the
    /// operator- or infrastructure-initiated terminal.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Published | Self::Cancelled)
    }

    /// Exceptional states — the job is blocked on something outside
    /// the normal flow.
    #[must_use]
    pub fn is_exceptional(self) -> bool {
        matches!(
            self,
            Self::HumanReviewRequired | Self::InfrastructureBlocked
        )
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl JobState {
    /// Stable, snake_case wire name for the variant.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::EventDetected => "event_detected",
            Self::Intake => "intake",
            Self::SourceReview => "source_review",
            Self::CandidateAssembly => "candidate_assembly",
            Self::SourceRevision => "source_revision",
            Self::SourceIntegrity => "source_integrity",
            Self::BuildValidation => "build_validation",
            Self::PackageQaValidation => "package_qa_validation",
            Self::FunctionalValidation => "functional_validation",
            Self::UpgradeValidation => "upgrade_validation",
            Self::ReleaseReview => "release_review",
            Self::FinalValidation => "final_validation",
            Self::ReadyForApproval => "ready_for_approval",
            Self::Approved => "approved",
            Self::PublicationPending => "publication_pending",
            Self::Published => "published",
            Self::HumanReviewRequired => "human_review_required",
            Self::InfrastructureBlocked => "infrastructure_blocked",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The durable identity of a maintenance job.
///
/// Immutable (§15: "Do not put mutable current workflow state directly
/// into this immutable domain identity"). The mutable projection is
/// [`JobProjection`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceJob {
    pub id: JobId,
    pub package: PackageIdentity,
    pub initiating_event: MaintenanceEventId,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "crate::json_schema_impls::Rfc3339DateTime")]
    pub created_at: OffsetDateTime,
}

impl MaintenanceJob {
    #[must_use]
    pub fn new(
        id: JobId,
        package: PackageIdentity,
        initiating_event: MaintenanceEventId,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            package,
            initiating_event,
            created_at,
        }
    }
}

/// A read-model projection of a [`MaintenanceJob`].
///
/// The state machine in `ironmaint-state` is the only writer of this
/// type (PHASE-0A.md §18: "Only `ironmaint-state::TransitionEngine`
/// may produce an accepted `StateTransitioned` event"). 0A.2 only
/// defines the value type; 0A.3 introduces the engine that mutates
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobProjection {
    pub job: MaintenanceJob,
    pub state: JobState,
    /// Currently active candidate, if any. `None` in early states
    /// (e.g. before `CandidateAssembly`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_candidate: Option<CandidateId>,
    /// Monotonic optimistic-concurrency token. Bumped by the state
    /// engine on every accepted transition (§17).
    pub version: u64,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "crate::json_schema_impls::Rfc3339DateTime")]
    pub updated_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{DistributionFamily, DistributionRef, DistributionRelease};
    use crate::package::PackageName;
    use std::collections::HashSet;
    use time::macros::datetime;

    fn pkg() -> PackageIdentity {
        PackageIdentity::new(
            DistributionRef::new(
                DistributionFamily::new("debian").unwrap(),
                DistributionRelease::new("unstable").unwrap(),
            ),
            PackageName::new("foo").unwrap(),
        )
    }

    #[test]
    fn job_state_has_19_distinct_variants() {
        let set: HashSet<_> = JobState::ALL.iter().copied().collect();
        assert_eq!(
            set.len(),
            JobState::ALL.len(),
            "JobState::ALL must contain only distinct variants",
        );
        // PHASE-0A.md §16 lists 19 names with `CandidateAssembly` duplicated;
        // after replacing the duplicate with `SourceRevision` we have 19
        // distinct variants.
        assert_eq!(set.len(), 19);
    }

    #[test]
    fn terminal_predicate_matches_published_and_cancelled() {
        assert!(JobState::Published.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        for s in JobState::ALL {
            if s != JobState::Published && s != JobState::Cancelled {
                assert!(!s.is_terminal(), "{:?} should not be terminal", s);
            }
        }
    }

    #[test]
    fn exceptional_predicate_matches_human_review_and_infrastructure_blocked() {
        assert!(JobState::HumanReviewRequired.is_exceptional());
        assert!(JobState::InfrastructureBlocked.is_exceptional());
        for s in JobState::ALL {
            if s != JobState::HumanReviewRequired && s != JobState::InfrastructureBlocked {
                assert!(!s.is_exceptional(), "{:?} should not be exceptional", s);
            }
        }
    }

    #[test]
    fn terminal_and_exceptional_are_disjoint() {
        for s in JobState::ALL {
            assert!(
                !(s.is_terminal() && s.is_exceptional()),
                "{:?} cannot be both terminal and exceptional",
                s
            );
        }
    }

    #[test]
    fn job_state_serializes_as_snake_case() {
        let json = serde_json::to_string(&JobState::PackageQaValidation).unwrap();
        assert_eq!(json, "\"package_qa_validation\"");
        let json = serde_json::to_string(&JobState::SourceRevision).unwrap();
        assert_eq!(json, "\"source_revision\"");
    }

    #[test]
    fn job_state_name_matches_serde_discriminator() {
        for s in JobState::ALL {
            let json = serde_json::to_string(&s).unwrap();
            let expected = format!("\"{}\"", s.name());
            assert_eq!(json, expected, "name() must match serde output for {s:?}");
        }
    }

    #[test]
    fn maintenance_job_construction() {
        let job = MaintenanceJob::new(
            JobId::new(),
            pkg(),
            crate::identity::MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        assert_eq!(job.package.source_name.as_str(), "foo");
    }

    #[test]
    fn maintenance_job_round_trip() {
        let job = MaintenanceJob::new(
            JobId::new(),
            pkg(),
            crate::identity::MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&job).unwrap();
        let parsed: MaintenanceJob = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, job);
    }

    #[test]
    fn job_projection_round_trip() {
        let job = MaintenanceJob::new(
            JobId::new(),
            pkg(),
            crate::identity::MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let proj = JobProjection {
            job,
            state: JobState::Intake,
            active_candidate: None,
            version: 1,
            updated_at: datetime!(2026-01-02 00:00:00 UTC),
        };
        let json = serde_json::to_string(&proj).unwrap();
        let parsed: JobProjection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, proj);
    }

    #[test]
    fn job_projection_omits_none_candidate() {
        let job = MaintenanceJob::new(
            JobId::new(),
            pkg(),
            crate::identity::MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let proj = JobProjection {
            job,
            state: JobState::EventDetected,
            active_candidate: None,
            version: 0,
            updated_at: datetime!(2026-01-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&proj).unwrap();
        assert!(!json.contains("active_candidate"));
    }
}
