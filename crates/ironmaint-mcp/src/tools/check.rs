//! `check.run` tool.

use ironmaint_core::JobId;
use ironmaint_executor::RetryClass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunCheckInput {
    pub job_id: JobId,
    pub tool_key: String,
    #[serde(default)]
    pub retry_class: Option<RetryClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunCheckOutput {
    pub tool_key: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
