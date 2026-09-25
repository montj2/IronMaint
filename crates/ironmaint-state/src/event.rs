//! Events emitted by [`crate::TransitionEngine`] (§18).
//!
//! [`StateTransitioned`] is the canonical "the engine accepted a
//! transition" event. [`JobEvent`] is the union over all events a
//! job emits during its lifetime; for 0A.3 only the state-transition
//! variant exists. Adapter- and policy-originated events
//! ([`JobEvent::Domain`]) are placeholders until 0A.4.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::{DomainEventId, JobProjection};

use crate::transition::Transition;

/// The event emitted by [`crate::TransitionEngine::apply`] on an
/// allowed transition.
///
/// `projection_after` carries the new [`JobProjection`] (state,
/// bumped version, `request.now` stamped on `updated_at`) so the
/// caller can persist it without recomputing the bumps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransitioned {
    pub transition: Transition,
    pub projection_after: JobProjection,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
}

impl StateTransitioned {
    #[must_use]
    pub fn new(
        transition: Transition,
        projection_after: JobProjection,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            transition,
            projection_after,
            occurred_at,
        }
    }
}

/// All events a job can emit.
///
/// 0A.3 only produces [`JobEvent::Transitioned`]. Domain events
/// (those produced by adapters and the policy engine outside the
/// state machine) are referenced by id in 0A.3; richer union members
/// land with the executor in 0B.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobEvent {
    Transitioned(StateTransitioned),
    Domain(DomainEventId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transition::Transition;
    use ironmaint_core::{
        DistributionFamily, DistributionRef, DistributionRelease, JobId, JobState, MaintenanceJob,
        PackageIdentity, PackageName,
    };
    use time::macros::datetime;

    fn projection(state: JobState, version: u64) -> JobProjection {
        let job = MaintenanceJob::new(
            JobId::new(),
            PackageIdentity::new(
                DistributionRef::new(
                    DistributionFamily::new("debian").unwrap(),
                    DistributionRelease::new("unstable").unwrap(),
                ),
                PackageName::new("foo").unwrap(),
            ),
            ironmaint_core::MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        JobProjection {
            job,
            state,
            active_candidate: None,
            version,
            updated_at: datetime!(2026-01-02 00:00:00 UTC),
        }
    }

    #[test]
    fn state_transitioned_construction() {
        let t = Transition {
            from: JobState::EventDetected,
            to: JobState::Intake,
            rule_index: 0,
        };
        let ev = StateTransitioned::new(
            t,
            projection(JobState::Intake, 1),
            datetime!(2026-01-02 00:00:00 UTC),
        );
        assert_eq!(ev.projection_after.version, 1);
        assert_eq!(ev.projection_after.state, JobState::Intake);
    }

    #[test]
    fn state_transitioned_round_trips() {
        let t = Transition {
            from: JobState::Intake,
            to: JobState::SourceReview,
            rule_index: 1,
        };
        let ev = StateTransitioned::new(
            t,
            projection(JobState::SourceReview, 5),
            datetime!(2026-03-03 03:03:03 UTC),
        );
        let json = serde_json::to_string(&ev).unwrap();
        let parsed: StateTransitioned = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ev);
    }

    #[test]
    fn job_event_transitioned_round_trips() {
        let t = Transition {
            from: JobState::EventDetected,
            to: JobState::Intake,
            rule_index: 0,
        };
        let ev = JobEvent::Transitioned(StateTransitioned::new(
            t,
            projection(JobState::Intake, 1),
            datetime!(2026-01-01 00:00:00 UTC),
        ));
        let json = serde_json::to_string(&ev).unwrap();
        let parsed: JobEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ev);
    }

    #[test]
    fn job_event_domain_carries_event_id() {
        let id = DomainEventId::new();
        let ev = JobEvent::Domain(id);
        let json = serde_json::to_string(&ev).unwrap();
        let parsed: JobEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ev);
    }
}
