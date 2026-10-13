//! `check.run` tool.

use ironmaint_core::{CheckId, JobId};
use ironmaint_evidence::EvidenceStatus;
use ironmaint_evidence::GateStatus;
use ironmaint_executor::RetryClass;
use ironmaint_runtime::outcome::CheckOutcome;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunCheckInput {
    pub job_id: JobId,
    /// The check to run, as advertised by `job.next_actions`.
    ///
    /// This replaced a `tool_key` field that the runtime's
    /// `RunCheck` command never accepted — three layers
    /// (`AllowedAction::RunCheck`, this input, and the runtime
    /// command) disagreed about the identity of a check, and only
    /// the runtime's `check_id` is one the store can resolve.
    /// `CheckStore` has no lookup-by-tool-key, so a tool key could
    /// not have been honoured even in principle.
    pub check_id: CheckId,
    #[serde(default)]
    pub retry_class: Option<RetryClass>,
}

/// The durable result of a check.
///
/// Deliberately does *not* carry `exit_code`/`stdout`/`stderr`.
/// Those are the executor's raw bytes: `exit_code` is persisted
/// nowhere, so the store could not corroborate it, and
/// `SKILL.md` forbids the agent from acting on raw tool output.
/// What survives a restart is the evidence row and the gate
/// verdict, so those are what the tool returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunCheckOutput {
    pub job_id: JobId,
    pub check_id: CheckId,
    /// The evidence row this run wrote, or `None` if the check has
    /// never been run.
    pub evidence_id: Option<ironmaint_core::EvidenceId>,
    /// The tool's own verdict, before gate aggregation.
    pub evidence_status: Option<EvidenceStatus>,
    /// The gate verdict after aggregation.
    pub gate_status: GateStatus,
    /// Whether the executor bounded stdout/stderr.
    pub truncated: bool,
}

impl From<CheckOutcome> for RunCheckOutput {
    fn from(o: CheckOutcome) -> Self {
        Self {
            job_id: o.job_id,
            check_id: o.check_id,
            evidence_id: o.evidence_id,
            evidence_status: o.evidence_status,
            gate_status: o.gate_status,
            truncated: o.truncated,
        }
    }
}
