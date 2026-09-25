//! `job.next_actions` tool.

use ironmaint_core::JobId;
use ironmaint_runtime::JobNextActions;
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
