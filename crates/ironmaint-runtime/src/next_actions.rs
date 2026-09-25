//! `JobNextActions` (PHASE-0B.md §42).
//!
//! Materialises the *next allowed moves* for the orchestrator:
//! the `AllowedAction`s it can take right now, and the
//! `ActionBlocker`s that explain why anything not listed is
//! off-limits.
//!
//! `ActionBlocker` is the runtime's own enum — distinct from
//! `ironmaint_state::TransitionBlocker`. The state machine's
//! blocker explains *why a transition cannot fire*; the
//! runtime's blocker explains *what is blocking the
//! orchestrator's next move*. Both can coexist on the same
//! job.

use ironmaint_core::JobId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AllowedAction {
    CaptureCandidate,
    RunCheck { tool_key: String },
    ApplyPatch,
    MarkObligationSatisfied,
    RequestApproval,
    AuthorizeOperation,
    Publish,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionBlocker {
    /// Job is already in a terminal state; no further moves.
    Terminal,
    /// A mandatory gate's evidence is still `NotEvaluated`.
    GatePending { tool_key: String },
    /// A mandatory gate's evidence is `Fail` and no exception
    /// is on file.
    GateFailed { tool_key: String },
    /// An obligation with `RequiresReview` is unsatisfied.
    ObligationPending { reference: String },
    /// The active candidate has not been captured yet.
    NoActiveCandidate,
    /// The state machine refused the transition; details are
    /// in the message.
    StateMachineBlocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobNextActions {
    pub job_id: JobId,
    pub allowed: Vec<AllowedAction>,
    pub blockers: Vec<ActionBlocker>,
}

impl JobNextActions {
    #[must_use]
    pub fn empty(job_id: JobId) -> Self {
        Self {
            job_id,
            allowed: vec![],
            blockers: vec![ActionBlocker::Terminal],
        }
    }
}
