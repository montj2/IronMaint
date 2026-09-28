//! Events emitted by [`crate::TransitionEngine`] (§18) and the
//! runtime (0B.4).
//!
//! [`StateTransitioned`] is the canonical "the engine accepted a
//! transition" event. [`JobEvent`] is the union over all events a
//! job emits during its lifetime. Three variants in 0B.4:
//!
//! - [`JobEvent::Transitioned`] — state-machine advance.
//! - [`JobEvent::Domain`] — adapter- or policy-originated event
//!   referenced by id (PHASE-0A §30).
//! - [`JobEvent::ToolRunFinished`] — runtime audit hook fired
//!   when a tool subprocess returns (PHASE-0B §15 last bullet,
//!   §65 partial). Does NOT advance the FSM: tool runs are
//!   observability signals, not state changes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::{DomainEventId, EvidenceId, JobProjection, SchemaVersion};

use crate::transition::Transition;

/// The event emitted by [`crate::TransitionEngine::apply`] on an
/// allowed transition.
///
/// `projection_after` carries the new [`JobProjection`] (state,
/// bumped version, `request.now` stamped on `updated_at`) so the
/// caller can persist it without recomputing the bumps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StateTransitioned {
    pub transition: Transition,
    pub projection_after: JobProjection,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "ironmaint_core::json_schema_impls::Rfc3339DateTime")]
    pub occurred_at: OffsetDateTime,
    /// Wire-format schema version (§60). Always serializes as
    /// `SchemaVersion::V1`; on parse, a missing field defaults to `V1`.
    #[serde(default)]
    pub schema_version: SchemaVersion,
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
            schema_version: SchemaVersion::default(),
        }
    }
}

/// Wire-format outcome of a tool subprocess (PHASE-0B.md §32, §92).
///
/// Mirrors the §92 exit-checkpoint taxonomy. Note this is a
/// *separate* enum from `ironmaint_executor::fixture::Outcome`:
/// the executor crate is downstream of state (per
/// `verify-architecture`) so state defines the canonical wire
/// vocabulary here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Pass,
    Fail,
    Timeout,
    Interrupted,
    InfrastructureFailed,
}

impl ToolOutcome {
    /// Stable wire-format name (snake_case).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
            Self::InfrastructureFailed => "infrastructure_failed",
        }
    }
}

impl std::fmt::Display for ToolOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Audit/observability event for "a tool subprocess finished
/// running" (PHASE-0B.md §15 last bullet, §65 partial).
///
/// `evidence_id` joins this event to the [`ironmaint_evidence::Evidence`]
/// row persisted by the runtime. `truncated` carries the same
/// `truncated = true` signal the executor observed (PHASE-0B.md §15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolRunFinished {
    pub evidence_id: EvidenceId,
    #[serde(default)]
    pub truncated: bool,
    pub outcome: ToolOutcome,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "ironmaint_core::json_schema_impls::Rfc3339DateTime")]
    pub occurred_at: OffsetDateTime,
    /// Wire-format schema version (§60). Always serializes as
    /// `SchemaVersion::V1`; on parse, a missing field defaults to `V1`.
    #[serde(default)]
    pub schema_version: SchemaVersion,
}

impl ToolRunFinished {
    #[must_use]
    pub fn new(
        evidence_id: EvidenceId,
        truncated: bool,
        outcome: ToolOutcome,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            evidence_id,
            truncated,
            outcome,
            occurred_at,
            schema_version: SchemaVersion::default(),
        }
    }
}

/// All events a job can emit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobEvent {
    Transitioned(StateTransitioned),
    Domain(DomainEventId),
    ToolRunFinished(ToolRunFinished),
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

    #[test]
    fn tool_outcome_round_trips_one_variant_each() {
        for v in [
            ToolOutcome::Pass,
            ToolOutcome::Fail,
            ToolOutcome::Timeout,
            ToolOutcome::Interrupted,
            ToolOutcome::InfrastructureFailed,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let back: ToolOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn tool_outcome_wire_format_is_snake_case() {
        assert_eq!(ToolOutcome::Pass.as_str(), "pass");
        assert_eq!(ToolOutcome::Fail.as_str(), "fail");
        assert_eq!(ToolOutcome::Timeout.as_str(), "timeout");
        assert_eq!(ToolOutcome::Interrupted.as_str(), "interrupted");
        assert_eq!(
            ToolOutcome::InfrastructureFailed.as_str(),
            "infrastructure_failed"
        );
    }

    #[test]
    fn tool_run_finished_event_round_trips() {
        let ev = JobEvent::ToolRunFinished(ToolRunFinished::new(
            EvidenceId::new(),
            true,
            ToolOutcome::Pass,
            datetime!(2026-04-04 04:04:04 UTC),
        ));
        let json = serde_json::to_string(&ev).unwrap();
        let parsed: JobEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ev);
    }

    #[test]
    fn tool_run_finished_truncated_defaults_to_false() {
        // Older wire payloads omit `truncated`; serde(default)
        // restores it to `false`. Drop the field and re-parse.
        let json = serde_json::json!({
            "tool_run_finished": {
                "evidence_id": EvidenceId::new().to_string(),
                "outcome": "pass",
                "occurred_at": "2026-01-01T00:00:00Z",
                "schema_version": 1u16,
            }
        })
        .to_string();
        let parsed: JobEvent = serde_json::from_str(&json).unwrap();
        if let JobEvent::ToolRunFinished(t) = parsed {
            assert!(!t.truncated);
        } else {
            panic!("expected ToolRunFinished variant, got {parsed:?}");
        }
    }
}
