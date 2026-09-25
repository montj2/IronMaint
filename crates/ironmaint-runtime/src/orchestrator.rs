//! `OrchestratorRef` (PHASE-0B.md §63): the per-job handle to
//! whoever is driving the workflow — IronClaw, a human reviewer,
//! or a custom orchestrator.
//!
//! Lives here (not in core) because it is per-orchestration,
//! not per-domain. It is a pure data type; no behaviour.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrchestratorKind {
    Ironclaw,
    Human,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrchestratorRef {
    pub kind: OrchestratorKind,
    /// External thread / session ID (e.g. IronClaw session id).
    #[serde(default)]
    pub external_thread: Option<String>,
    /// External job / conversation reference.
    #[serde(default)]
    pub external_job: Option<String>,
}

impl OrchestratorRef {
    #[must_use]
    pub fn ironclaw() -> Self {
        Self {
            kind: OrchestratorKind::Ironclaw,
            external_thread: None,
            external_job: None,
        }
    }
}
