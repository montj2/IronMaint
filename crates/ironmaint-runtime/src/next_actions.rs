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

use ironmaint_core::{CheckId, JobId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AllowedAction {
    CaptureCandidate,
    RunCheck { check_id: CheckId },
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
    ///
    /// `tool_key` is `None` when the job has no materialised check
    /// for the pending gate, so there is no tool to name. It used
    /// to be a hardcoded `"synthetic.build.validate"`, which put a
    /// fixture string into a domain projection: a real job with no
    /// adapter behind it was told to run a tool that does not
    /// exist. The honest answer when nothing is materialised is to
    /// say so, and the handler fills this in from the store when a
    /// check does exist.
    GatePending { tool_key: Option<String> },
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
