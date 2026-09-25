//! `job.create` and `job.get` tools.

use ironmaint_core::{JobId, PackageIdentity};
use ironmaint_runtime::OrchestratorRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateJobInput {
    pub orchestrator: OrchestratorRef,
    pub package: PackageIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateJobOutput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetJobInput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetJobOutput {
    pub projection: serde_json::Value,
}
