//! `job.next_actions`, `job.reconcile`, and `job.resume` tools.

use ironmaint_core::{JobId, JobState};
use ironmaint_runtime::{JobNextActions, ReconcileOutcome};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NextActionsInput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NextActionsOutput {
    pub actions: JobNextActions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileInput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileOutput {
    pub outcome: ReconcileOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResumeInput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResumeOutput {
    /// The state the job was returned to, read back from the
    /// projection rather than echoed from the command's
    /// side-effect strings — the same reasoning `check.run`
    /// follows when it reports its outcome.
    pub resumed_to: JobState,
}
