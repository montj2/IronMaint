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
//!
//! 0B.10 added two more:
//!
//! - [`JobEvent::ResumeRecorded`] — the record 0A §21 requires
//!   before a job may leave an exceptional state. Also does NOT
//!   advance the FSM: it is written at the moment of *entering*
//!   the exceptional state, and names the state to return to.
//! - [`JobEvent::JobCreated`] — the job's first event, carrying the
//!   initial projection. It is the seed a replay starts from, so
//!   that "rebuild from events" is true for a job that has never
//!   transitioned and not only for one that has.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::{DomainEventId, EvidenceId, JobProjection, SchemaVersion};

use crate::transition::{ResumeRecord, Transition};

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

/// The durable record of a candidate becoming the job's active one.
///
/// A separate struct rather than inline variant fields, so the SQLite
/// backend can `encode_json` / `map_json` it exactly as it does
/// `ToolRunFinished` and `ResumeRecord`. Hand-rolling the JSON in one
/// direction and parsing it in the other is how a payload ends up
/// asymmetric, and nothing but a round-trip test would notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CandidateActivated {
    pub candidate_id: ironmaint_core::CandidateId,
    /// The immutable identity of the source revision being activated.
    /// Carried alongside the id so an audit reader can tell *which*
    /// tree was made active without joining back to the candidates
    /// table — the same self-sufficiency argument that keeps this
    /// event out of the log's shadow.
    pub fingerprint: ironmaint_core::CandidateFingerprint,
    /// The projection version the activation left behind. Activation
    /// is a CAS write (`put_projection` with `current.version`), so
    /// without this the replay cannot know whether its write would
    /// have been accepted.
    pub version_after: u64,
    #[schemars(with = "ironmaint_core::json_schema_impls::Rfc3339DateTime")]
    pub updated_at: OffsetDateTime,
}

/// All events a job can emit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobEvent {
    /// The job's first event, and the seed every projection replay
    /// starts from.
    ///
    /// It carries the whole initial `JobProjection` rather than a
    /// reference, because nothing before it exists. Without this
    /// variant the log could not reconstruct a job that had never
    /// transitioned: the projection's contents lived only in the
    /// `put_projection` write, and a `Domain` event carries just an
    /// id. That made §102 item 3 — "Job projections can be rebuilt
    /// from events" — false for every real job, and left
    /// `ironmaintctl rebuild-projections` (§36's operator escape
    /// hatch) reporting every job as an error on a real database.
    ///
    /// A projection is not a transition, so this is not modelled as
    /// one: it has no `from`/`to` and never advances the FSM. It is
    /// the record that a job came into being, and what it looked
    /// like when it did.
    JobCreated(ironmaint_core::JobProjection),
    Transitioned(StateTransitioned),
    Domain(DomainEventId),
    ToolRunFinished(ToolRunFinished),
    ResumeRecorded(ResumeRecord),
    /// A candidate became the job's active candidate.
    ///
    /// Activation is not a transition — it does not move the FSM and
    /// has no `from`/`to` — but it is not a bare domain back-reference
    /// either, and that is what this variant exists to fix. D-16:
    /// `activate_candidate` set `active_candidate`, bumped `version`
    /// and stamped `updated_at` in the **projection row**, then
    /// appended a `Domain` id. `ProjectionApply::apply` treats
    /// `Domain` as a no-op, so the log recorded *that* something
    /// happened and the row recorded *what* — and the two disagreed
    /// by construction on every activation.
    ///
    /// §36 makes the log the authority for a projection, and §30 binds
    /// every gate verdict to the active candidate, so a replay that
    /// drops this binding does not degrade the row — it invalidates
    /// the evidence for every check already run against it.
    ///
    /// The three fields are exactly what the row gained. `job_id` is
    /// not carried: the envelope already has it, and duplicating it
    /// would let the two disagree. `Domain` is deliberately *not*
    /// widened — it is used for several unrelated domain writes and
    /// its bare id is an audit back-reference, not a state record.
    CandidateActivated(CandidateActivated),
}

impl JobEvent {
    /// The projection a replay should *start* from, if this event
    /// carries one.
    ///
    /// Exactly two variants do: `JobCreated` (the normal first
    /// event) and `Transitioned` (a log written before 0B.10, whose
    /// seed was the post-transition projection). The other three are
    /// records — a reference, an observability signal, a resume
    /// target — and none of them describes a projection, which is
    /// why a log beginning with one genuinely cannot be rebuilt.
    ///
    /// The `Err` is the reason, phrased for an operator reading
    /// `ironmaintctl rebuild-projections` output. It lives here
    /// rather than in each store because the two backends had
    /// drifted into separate copies of this decision, and only one
    /// of them was ever exercised.
    pub fn seed_projection(&self) -> Result<ironmaint_core::JobProjection, String> {
        match self {
            Self::JobCreated(p) => Ok(p.clone()),
            Self::Transitioned(t) => Ok(t.projection_after.clone()),
            Self::Domain(_) => Err("first event is a Domain reference".to_string()),
            Self::ToolRunFinished(_) => Err("first event is a ToolRunFinished".to_string()),
            Self::ResumeRecorded(_) => Err("first event is a ResumeRecorded".to_string()),
            // Structurally impossible rather than merely unlikely: a
            // candidate is activated only after `CaptureCandidate`
            // has persisted it, which requires a `JobCreated` seed to
            // attach the job to. This arm exists so that stays a
            // compile-time obligation — a future variant added here
            // without deciding which side of that line it falls on
            // should not compile.
            Self::CandidateActivated(_) => Err("first event is a CandidateActivated".to_string()),
        }
    }
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
