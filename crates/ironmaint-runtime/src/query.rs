//! Runtime queries (read-only).

use ironmaint_core::JobId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeQuery {
    GetJob { job_id: JobId },
    GetProjection { job_id: JobId },
    ListNextActions { job_id: JobId },
}
