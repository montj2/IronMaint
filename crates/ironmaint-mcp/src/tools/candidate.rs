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
    /// What the capture did beyond persisting the candidate.
    ///
    /// Capture activates the candidate and derives its gates and
    /// obligations (PHASE-0B.md §67, §45, §48), and each of those
    /// steps can legitimately do nothing — no adapter registered for
    /// the family, an adapter that plans no checks, a job too far
    /// along to activate a new candidate. An agent that cannot see
    /// which happened cannot tell "my capture was inert" from "my
    /// capture worked", and the first is the one that needs
    /// diagnosing.
    pub notes: Vec<String>,
}
