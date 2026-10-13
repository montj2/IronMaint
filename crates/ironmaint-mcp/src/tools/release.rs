//! `release.candidate.create` tool — the §42 snapshot.

use ironmaint_core::JobId;
use ironmaint_policy::ReleaseCandidate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateInput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateOutput {
    /// The snapshot bound to the job's active candidate.
    ///
    /// Idempotent, so this is the same value whether this call
    /// assembled it or found an earlier one. There is deliberately
    /// no "did I create it" flag: the command reports that in a
    /// side-effect string, and putting a boolean derived from parsing
    /// that string on the wire would be a second, weaker account of
    /// what happened — the same reason `check.run` and `job.resume`
    /// read the durable state back instead.
    pub release_candidate: ReleaseCandidate,
}
