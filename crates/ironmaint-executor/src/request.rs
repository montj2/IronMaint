//! Execution request type.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::JobId;
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
    /// The job this invocation belongs to.
    ///
    /// Required, and required *here* rather than passed separately
    /// to `execute`, because §15's retention cap is per-job: the
    /// executor asks a `JobArtifactGuardFactory` for this job's
    /// guard, and a request that did not say which job would be a
    /// request whose artifacts could not be budgeted at all. This
    /// field was absent, and its absence is why `ProcessExecutor`
    /// held a `guard_factory` it could not call — D-08.
    pub job_id: JobId,
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
        job_id: JobId,
        tool_key: ToolCapabilityKey,
        retry_class: RetryClass,
        input: serde_json::Value,
    ) -> Self {
        Self {
            job_id,
            tool_key,
            retry_class,
            input,
            env_overrides: std::collections::BTreeMap::new(),
        }
    }
}
