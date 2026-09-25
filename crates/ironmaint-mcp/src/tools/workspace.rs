//! `workspace.apply_patch` and `workspace.stat` tools.

use ironmaint_core::JobId;
use ironmaint_workspace::WorkspaceRevision;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyPatchInput {
    pub job_id: JobId,
    pub patch: String,
    pub expected_revision: WorkspaceRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyPatchOutput {
    pub new_revision: WorkspaceRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatInput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatOutput {
    pub revision: WorkspaceRevision,
}
