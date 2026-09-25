//! Build capability (§49) and tool capability keys (§50).
//!
//! The adapter does not execute commands — it produces plans
//! describing what gates / tools must run, and the executor
//! materializes them.

use ironmaint_evidence::EvidenceKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contexts::CandidateContext;
use crate::error::AdapterError;
use crate::tool_key::ToolCapabilityKey;

/// A single planned gate / check (§49).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlannedCheck {
    pub key: ToolCapabilityKey,
    pub evidence_kind: EvidenceKind,
    pub mandatory: bool,
}

impl PlannedCheck {
    #[must_use]
    pub fn new(key: ToolCapabilityKey, evidence_kind: EvidenceKind, mandatory: bool) -> Self {
        Self {
            key,
            evidence_kind,
            mandatory,
        }
    }
}

/// Bundle of [`PlannedCheck`]s for one build phase (§49).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct BuildPlan {
    pub checks: Vec<PlannedCheck>,
}

impl BuildPlan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_check(mut self, check: PlannedCheck) -> Self {
        self.checks.push(check);
        self
    }
}

/// Bundle of [`PlannedCheck`]s for one QA phase (§49).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct QaPlan {
    pub checks: Vec<PlannedCheck>,
}

impl QaPlan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_check(mut self, check: PlannedCheck) -> Self {
        self.checks.push(check);
        self
    }
}

/// Optional capability: produce build / QA plans (§49).
pub trait BuildCapability: Send + Sync + 'static {
    /// Plan build-phase checks.
    ///
    /// # Errors
    /// Returns `AdapterError { kind: Unsupported, .. }` if the
    /// adapter cannot describe a build plan for the candidate.
    fn build_plan(&self, context: &CandidateContext) -> Result<BuildPlan, AdapterError>;

    /// Plan QA-phase checks.
    fn qa_plan(&self, context: &CandidateContext) -> Result<QaPlan, AdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_key::ToolCapabilityKey;

    #[test]
    fn planned_check_round_trip_through_serde_via_enum() {
        // Smoke check that the types compose. The serde round-trip
        // itself is at the JSON snapshot layer (0A.6).
        let check = PlannedCheck::new(
            ToolCapabilityKey::new("debian.build.sbuild").unwrap(),
            EvidenceKind::Build,
            true,
        );
        let plan = BuildPlan::new().with_check(check);
        assert_eq!(plan.checks.len(), 1);
        assert!(plan.checks[0].mandatory);
    }

    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn BuildCapability>();
    }
}
