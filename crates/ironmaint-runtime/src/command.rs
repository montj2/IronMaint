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
    /// Ask a human to approve something.
    ///
    /// **Always refused** with [`RuntimeErrorKind::Unsupported`](crate::error::RuntimeErrorKind::Unsupported).
    /// The command exists so the refusal is *observable*: an
    /// orchestrator that reaches `ReadyForApproval` gets a typed
    /// answer naming the boundary, rather than a missing-arms
    /// `Other("unhandled command")` or a silent no-op that looks
    /// like a queued request.
    ///
    /// Why refusal rather than implementation: an approval is a
    /// human principal's decision, and the caller of this command
    /// is the agent. 0B ships no approval principal, no
    /// out-of-band delivery channel, and no durable
    /// `ApprovalStore` — implementing the write would mean
    /// recording a decision nobody made. Until those exist the
    /// honest answer is that the capability does not exist, and
    /// `reconcile` still reports `NeedsActorDecision` at
    /// `ReadyForApproval` so the job parks rather than advancing.
    ///
    /// `next_actions` still advertises `RequestApproval` at that
    /// state, and that is not a contradiction: asking for approval
    /// *is* the orchestrator's next move, and the refusal is the
    /// answer. What the runtime refuses is recording the request as
    /// though a decision had been made. An agent that follows
    /// `next_actions`, calls this, and reads the refusal has learned
    /// it must hand off to a person — which is the exit checkpoint
    /// in `SKILL.md` made executable rather than implied.
    ///
    /// The field is carried so the request is well-formed and a
    /// future implementation has the category it would need; it is
    /// never inspected today, and the refusal message echoes it so
    /// the caller learns which approval was being asked for.
    RequestApproval {
        job_id: JobId,
        category: ironmaint_policy::ApprovalCategory,
    },
    /// Walk the static rule table and advance state until a
    /// blocker, an exceptional state, an actor-required
    /// transition, or a concurrent-modification error is hit
    /// (§41). Returns a [`ReconcileOutcome`] describing where
    /// the walk stopped.
    Reconcile { job_id: JobId },
}
