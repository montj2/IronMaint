//! Result normalizer — converts a raw `ExecutionRecord` into the
//! structured form the runtime persists as evidence
//! (PHASE-0B.md §28).
//!
//! The trait lives in `ironmaint-executor` because it consumes
//! the executor-owned `ExecutionRecord` type. Implementations
//! can reference `ironmaint-evidence::EvidenceStatus` and friends
//! through the existing `executor → evidence` edge.

use ironmaint_evidence::EvidenceStatus;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::record::ExecutionRecord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Observation {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NormalizedResult {
    pub evidence_status: EvidenceStatus,
    #[serde(default)]
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub invalidations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NormalizationError {
    #[error("record is malformed: {0}")]
    Malformed(String),
    #[error("normalizer cannot classify record: {0}")]
    Unclassifiable(String),
}

pub trait ResultNormalizer: Send + Sync {
    fn normalize(&self, record: &ExecutionRecord) -> Result<NormalizedResult, NormalizationError>;
}
