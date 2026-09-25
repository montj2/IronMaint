//! Runtime commands (mutate state).
//!
//! Every command goes through `RuntimeService::handle_command`.
//! The service is responsible for going through the state
//! machine; commands never write to the store directly.

use ironmaint_core::{CandidateFingerprint, JobId, PackageIdentity};
use ironmaint_evidence::{EvidenceKind, EvidenceStatus};
use ironmaint_executor::RetryClass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeCommand {
    CreateJob {
        orchestrator: crate::orchestrator::OrchestratorRef,
        package: PackageIdentity,
    },
    SetActiveCandidate {
        job_id: JobId,
        fingerprint: CandidateFingerprint,
    },
    RecordCheckEvidence {
        job_id: JobId,
        tool_key: String,
        evidence_kind: EvidenceKind,
        status: EvidenceStatus,
        producer: String,
    },
    MarkObligationSatisfied {
        job_id: JobId,
        obligation_ref: String,
    },
    RunCheck {
        job_id: JobId,
        tool_key: String,
        retry_class: RetryClass,
    },
}
