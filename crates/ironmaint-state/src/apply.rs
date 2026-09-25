//! Pure event-replay helper for `JobProjection`.
//!
//! `JobProjection` lives in `ironmaint-core` (the bottom of the
//! domain dependency graph). It cannot depend on `JobEvent` (which
//! lives here in `ironmaint-state`) without creating a cycle. The
//! `ProjectionApply` extension trait provides the replay method on
//! `JobProjection` so the event store can rebuild projections
//! without depending on the engine directly.

use ironmaint_core::JobProjection;
use time::OffsetDateTime;

use crate::JobEvent;

/// Pure replay: reconstruct the projection after applying one
/// `JobEvent`. Mirrors the shape produced by
/// [`crate::TransitionEngine::apply`].
///
/// `JobEvent::Domain(_)` references a domain event by id rather
/// than carrying its payload inline (PHASE-0A §30); for replay
/// purposes this is a no-op — the domain event's effects on the
/// projection are recorded as a subsequent `Transitioned` event
/// once the engine processes them.
///
/// `occurred_at` is supplied by the caller (typically the
/// envelope's `occurred_at` field) so this method stays
/// allocation-free and does not read the clock.
pub trait ProjectionApply {
    fn apply(&self, event: &JobEvent, occurred_at: OffsetDateTime) -> JobProjection;
}

impl ProjectionApply for JobProjection {
    fn apply(&self, event: &JobEvent, occurred_at: OffsetDateTime) -> JobProjection {
        match event {
            JobEvent::Transitioned(t) => JobProjection {
                job: self.job.clone(),
                state: t.transition.to,
                active_candidate: t.projection_after.active_candidate,
                version: t.projection_after.version,
                updated_at: occurred_at,
            },
            JobEvent::Domain(_) => self.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{
        CandidateId, DistributionFamily, DistributionRef, DistributionRelease, JobState,
        MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName,
    };
    use time::macros::datetime;

    use crate::StateTransitioned;
    use crate::Transition;

    fn fixture_projection() -> JobProjection {
        JobProjection {
            job: MaintenanceJob::new(
                ironmaint_core::JobId::new(),
                PackageIdentity::new(
                    DistributionRef::new(
                        DistributionFamily::new("debian").unwrap(),
                        DistributionRelease::new("unstable").unwrap(),
                    ),
                    PackageName::new("foo").unwrap(),
                ),
                MaintenanceEventId::new(),
                datetime!(2026-01-01 00:00:00 UTC),
            ),
            state: JobState::EventDetected,
            active_candidate: None,
            version: 0,
            updated_at: datetime!(2026-01-01 00:00:00 UTC),
        }
    }

    #[test]
    fn transitioned_event_advances_state_and_version() {
        let proj = fixture_projection();
        let occurred = datetime!(2026-01-02 00:00:00 UTC);
        let transitioned = StateTransitioned::new(
            Transition {
                from: JobState::EventDetected,
                to: JobState::Intake,
                rule_index: 0,
            },
            JobProjection {
                job: proj.job.clone(),
                state: JobState::Intake,
                active_candidate: None,
                version: 1,
                updated_at: occurred,
            },
            occurred,
        );
        let next = proj.apply(&JobEvent::Transitioned(transitioned), occurred);
        assert_eq!(next.state, JobState::Intake);
        assert_eq!(next.version, 1);
        assert_eq!(next.updated_at, occurred);
    }

    #[test]
    fn domain_event_is_a_no_op() {
        let proj = fixture_projection();
        let before = proj.clone();
        let next = proj.apply(
            &JobEvent::Domain(ironmaint_core::DomainEventId::new()),
            datetime!(2026-02-01 00:00:00 UTC),
        );
        assert_eq!(next, before);
    }

    #[test]
    fn active_candidate_is_taken_from_projection_after() {
        let proj = fixture_projection();
        let cand = CandidateId::new();
        let occurred = datetime!(2026-01-02 00:00:00 UTC);
        let transitioned = StateTransitioned::new(
            Transition {
                from: JobState::Intake,
                to: JobState::CandidateAssembly,
                rule_index: 1,
            },
            JobProjection {
                job: proj.job.clone(),
                state: JobState::CandidateAssembly,
                active_candidate: Some(cand),
                version: 2,
                updated_at: occurred,
            },
            occurred,
        );
        let next = proj.apply(&JobEvent::Transitioned(transitioned), occurred);
        assert_eq!(next.active_candidate, Some(cand));
    }
}
