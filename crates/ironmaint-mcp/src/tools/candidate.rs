//! `candidate.capture` tool.

use ironmaint_core::{CandidateFingerprint, JobId, PackageIdentity};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CaptureInput {
    pub job_id: JobId,
    pub package: PackageIdentity,
    pub repository_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CaptureOutput {
    pub fingerprint: CandidateFingerprint,
}
