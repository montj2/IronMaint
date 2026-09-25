//! Execution request type.

use ironmaint_adapter_api::ToolCapabilityKey;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::retry::RetryClass;

/// Inputs to one tool invocation (PHASE-0B.md §22).
///
/// The executor receives an `ExecutionRequest`, looks up the
/// registered capability, and dispatches. The `input` payload is
/// opaque JSON; the tool definition is responsible for parsing
/// its own shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionRequest {
    pub tool_key: ToolCapabilityKey,
    pub retry_class: RetryClass,
    pub input: serde_json::Value,
    /// Optional environment overrides; merged into the sanitised
    /// process environment. Tool definitions must reject keys
    /// that aren't allow-listed.
    #[serde(default)]
    pub env_overrides: std::collections::BTreeMap<String, String>,
}

impl ExecutionRequest {
    #[must_use]
    pub fn new(
        tool_key: ToolCapabilityKey,
        retry_class: RetryClass,
        input: serde_json::Value,
    ) -> Self {
        Self {
            tool_key,
            retry_class,
            input,
            env_overrides: std::collections::BTreeMap::new(),
        }
    }
}
