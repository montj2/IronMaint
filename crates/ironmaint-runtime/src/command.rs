//! Runtime commands (mutate state).
//!
//! Every command goes through `RuntimeService::handle_command`.
//! The service is responsible for going through the state
//! machine; commands never write to the store directly.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{CandidateFingerprint, CheckId, JobId, PackageIdentity, SourceCandidate};
use ironmaint_evidence::{EvidenceKind, EvidenceStatus};
use ironmaint_executor::RetryClass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
// `SourceCandidate` is the largest variant by a wide margin, but it
// is the canonical command payload from PHASE-0B.md §22 / §45.
// Boxing it would change the wire shape (a `Box<SourceCandidate>`
// serialises identically, but the in-memory size trade-off is not
// worth the indirection given how rarely these commands cross the
// dispatch boundary). Allow the lint explicitly.
#[allow(clippy::large_enum_variant)]
pub enum RuntimeCommand {
    CreateJob {
        orchestrator: crate::orchestrator::OrchestratorRef,
        package: PackageIdentity,
    },
    /// Persist a freshly minted `SourceCandidate` for a job.
    ///
    /// Idempotent on `(job_id, fingerprint)`: a second call with
    /// the same fingerprint returns the original `CandidateId`
    /// without re-inserting. The candidate is **not** activated
    /// here — that is `SetActiveCandidate`'s job. This split
    /// matches PHASE-0B.md §6 (capture boundary) vs §45 (check
    /// planning boundary).
    CaptureCandidate {
        job_id: JobId,
        candidate: SourceCandidate,
    },
    /// Materialise an adapter's `BuildPlan` + `QaPlan` into durable
    /// `CheckDefinition`s tied to the active candidate.
    ///
    /// Each `(tool_key, evidence_kind, mandatory)` triple becomes a
    /// `CheckDefinition` row plus a corresponding `GateDefinition`.
    /// The handler is pure projection materialisation — it does
    /// not transition state, only writes durable check contracts
    /// that `RunCheck { check_id }` and `next_actions` will
    /// reference.
    MaterializeChecks {
        job_id: JobId,
        candidate: CandidateFingerprint,
        planned: Vec<(ToolCapabilityKey, EvidenceKind, bool)>,
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
        /// Resolved check to execute. The runtime looks up the
        /// [`CheckDefinition`](ironmaint_store::CheckDefinition)
        /// by id, executes the bound tool via the executor, and
        /// records the resulting evidence against the gate.
        check_id: CheckId,
        retry_class: RetryClass,
        // The job_id is implied by `check_id.job_id`; the
        // explicit `job_id` keeps the dispatch shape uniform
        // across commands.
        job_id: JobId,
    },
    /// Walk the static rule table and advance state until a
    /// blocker, an exceptional state, an actor-required
    /// transition, or a concurrent-modification error is hit
    /// (§41). Returns a [`ReconcileOutcome`] describing where
    /// the walk stopped.
    Reconcile { job_id: JobId },
}
