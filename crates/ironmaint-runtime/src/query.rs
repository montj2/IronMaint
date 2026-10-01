//! Runtime queries (read-only).

use ironmaint_core::{CheckId, JobId, OperationId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeQuery {
    GetJob {
        job_id: JobId,
    },
    GetProjection {
        job_id: JobId,
    },
    ListNextActions {
        job_id: JobId,
    },
    /// Read back what a check produced: the evidence row it wrote
    /// and the gate verdict that evidence contributed to.
    GetCheckOutcome {
        check_id: CheckId,
    },
    /// Read a `PrivilegedOperation` by id.
    ///
    /// Routed through the runtime rather than a store handle the
    /// MCP layer holds, per PHASE-0B.md §98.6: the MCP surface
    /// reaches persistence through the runtime, never around it.
    GetOperation {
        operation_id: OperationId,
    },
    /// The release candidate for a job's *active* source candidate.
    ///
    /// Addressed by job rather than by id because a `JobId` is what
    /// a caller has: the id is minted by
    /// [`crate::RuntimeCommand::CreateReleaseCandidate`], so a
    /// caller that arrived first cannot know it. Since that command
    /// is idempotent on (job, fingerprint) there is at most one
    /// answer, and asking for it is how a caller finds out whether
    /// one has been assembled at all.
    GetReleaseCandidate {
        job_id: JobId,
    },
}
