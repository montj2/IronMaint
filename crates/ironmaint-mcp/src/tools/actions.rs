//! `job.next_actions` and `job.reconcile` tools.

use ironmaint_core::JobId;
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
