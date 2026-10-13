//! Phase 0B.5 C5 — `Reconcile` (§41).
//!
//! Walks the static [`TRANSITION_RULES`](ironmaint_state::TRANSITION_RULES)
//! table for a job's current state, advancing the projection
//! through every rule whose requirements are satisfied by the
//! evidence ledger, gate definitions, and obligation set.
//! Stops on the first blocker, on an exceptional state, on a
//! rule whose requirements demand an actor decision
//! (`ReadyForApproval → Approved`, which requires an approval
//! decision), or on a concurrent-modification error.
//!
//! The walker is bounded: at most 15 iterations (one per
//! static rule in the §20 forward chain). If the loop
//! somehow runs longer than that — a defensive bound only —
//! it returns `NoOp` rather than spinning forever.

use ironmaint_core::{JobId, JobState};
use ironmaint_state::TransitionBlocker;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Format a single [`TransitionBlocker`] as a `String` for use
/// in [`ReconcileOutcome`]. The state-machine blocker enum is
/// intentionally not serializable on the wire — its details
/// travel through [`crate::RuntimeError::InvalidInput`] and
/// out to the orchestrator through the runtime error surface.
#[must_use]
pub fn format_blocker(b: &TransitionBlocker) -> String {
    format!("{b:?}")
}

/// Outcome of a `reconcile` invocation (§41).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileOutcome {
    /// The projection advanced through one or more rules
    /// and stopped on its own (terminal, exceptional, or
    /// actor-gated).
    Advanced {
        from: JobState,
        to: JobState,
        rule_index: usize,
    },
    /// The projection did not move.
    NoOp { current: JobState },
    /// The next rule was blocked; the blockers are reported.
    Blocked {
        current: JobState,
        target: JobState,
        blockers: Vec<String>,
    },
    /// The next rule requires an actor decision (e.g. an
    /// approval category); the runtime cannot advance
    /// without one.
    NeedsActorDecision {
        current: JobState,
        target: JobState,
        blockers: Vec<String>,
    },
    /// The job is in an exceptional state that requires
    /// external intervention.
    Exceptional { current: JobState },
    /// Optimistic-concurrency contention: another writer
    /// advanced the projection between read and write.
    /// The caller should reload and retry.
    ConcurrentModification,
}

const _: fn() = || {
    let _: JobId = JobId::new(); // ensure JobId is referenced for documentation purposes
};
