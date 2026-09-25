//! Event envelope (PHASE-0B.md §56).
//!
//! The wire format for events that the durable event store appends
//! to its per-job log. Wraps an [`ironmaint_state::JobEvent`] with
//! the metadata required for:
//!
//! - Idempotency / replay safety ([`event_id`] is a UUIDv7)
//! - Per-job monotonic ordering ([`sequence`] is 1-indexed, set by
//!   the producer)
//! - Causality tracking ([`occurred_at`] is RFC3339 UTC)
//!
//! EventEnvelope is defined here (in `ironmaint-store`) rather than
//! `ironmaint-state` because:
//!
//! - 0A.3 only has `JobEvent`; the envelope wrapper is a 0B
//!   concern (the executor / runtime assign sequence numbers).
//! - Keeping it in `ironmaint-store` means the dependency direction
//!   is clean: state defines the inner union, store defines the
//!   durable wrapper, runtime reads / writes the envelope.
//!
//! [`event_id`]: EventEnvelope::event_id

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::JobId;
use ironmaint_core::MaintenanceEventId;
use ironmaint_state::JobEvent;

/// A single event in the per-job durable event log.
///
/// `sequence` is monotonically increasing per `job_id`; the store
/// rejects out-of-order appends with
/// [`crate::error::StoreErrorKind::SequenceOutOfRange`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventEnvelope {
    /// Unique across all events; UUIDv7 for time-orderable ids.
    pub event_id: MaintenanceEventId,
    /// Job this event belongs to.
    pub job_id: JobId,
    /// 1-indexed sequence number, monotonic per `job_id`.
    pub sequence: u64,
    /// RFC3339 UTC.
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "ironmaint_core::json_schema_impls::Rfc3339DateTime")]
    pub occurred_at: OffsetDateTime,
    /// The actual event payload.
    pub event: JobEvent,
}

impl EventEnvelope {
    /// Construct an envelope. Callers are responsible for assigning
    /// `sequence` (the store enforces monotonicity on append).
    #[must_use]
    pub fn new(
        event_id: MaintenanceEventId,
        job_id: JobId,
        sequence: u64,
        occurred_at: OffsetDateTime,
        event: JobEvent,
    ) -> Self {
        Self {
            event_id,
            job_id,
            sequence,
            occurred_at,
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::JobState;

    #[test]
    fn envelope_round_trips_through_json() {
        let env = EventEnvelope::new(
            MaintenanceEventId::new(),
            JobId::new(),
            1,
            OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            JobEvent::Transitioned(ironmaint_state::StateTransitioned::new(
                ironmaint_state::Transition {
                    from: JobState::EventDetected,
                    to: JobState::Intake,
                    rule_index: 0,
                },
                ironmaint_core::JobProjection {
                    job: ironmaint_core::MaintenanceJob::new(
                        JobId::new(),
                        ironmaint_core::PackageIdentity::new(
                            ironmaint_core::DistributionRef::new(
                                ironmaint_core::DistributionFamily::new("debian").unwrap(),
                                ironmaint_core::DistributionRelease::new("unstable").unwrap(),
                            ),
                            ironmaint_core::PackageName::new("foo").unwrap(),
                        ),
                        MaintenanceEventId::new(),
                        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                    ),
                    state: JobState::Intake,
                    active_candidate: None,
                    version: 1,
                    updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                },
                OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            )),
        );
        let json = serde_json::to_string(&env).unwrap();
        let parsed: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
