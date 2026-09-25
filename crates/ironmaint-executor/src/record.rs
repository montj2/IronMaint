//! Execution record — the durable, JSON-serialised result of a
//! tool invocation.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::json_schema_impls::Rfc3339DateTime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::retry::RetryClass;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionRecord {
    pub tool_key: ToolCapabilityKey,
    pub retry_class: RetryClass,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "Rfc3339DateTime")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "Rfc3339DateTime")]
    pub finished_at: OffsetDateTime,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Whether the executor's retry budget was exhausted.
    #[serde(default)]
    pub retries_exhausted: bool,
}

impl ExecutionRecord {
    #[must_use]
    pub fn duration_ms(&self) -> i64 {
        let ms = (self.finished_at - self.started_at).whole_milliseconds();
        if ms <= 0 {
            0
        } else {
            ms.min(i64::MAX as i128) as i64
        }
    }

    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.exit_code == 0
    }
}
