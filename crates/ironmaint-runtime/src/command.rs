//! Runtime commands (mutate state).
//!
//! Every command goes through `RuntimeService::handle_command`.
//! The service is responsible for going through the state
//! machine; commands never write to the store directly.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{CandidateFingerprint, CheckId, JobId, PackageIdentity, SourceCandidate};
use ironmaint_evidence::{EvidenceKind, EvidenceStatus};
use ironmaint_executor::RetryClass;
use ironmaint_policy::ObligationOutcome;
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
    /// Record the verdict of evaluating one obligation (§34).
    ///
    /// Replaces a narrower `MarkObligationSatisfied`, which could only
    /// ever write `Pass`. That made a policy evaluation returning a
    /// negative verdict **unrepresentable** in production: the engine
    /// has branches for `Fail` and `RequiresReview` and blocked on
    /// them, and nothing outside a test could ever put an obligation
    /// into either. A state machine modelling something the system
    /// cannot produce is the same defect 0B.10 C1 found three times
    /// over, and a fourth time here.
    ///
    /// The `outcome` is an [`ObligationOutcome`], not a raw
    /// `ObligationStatus`: `NotEvaluated` is an initial state rather
    /// than a verdict, and `ExceptionApproved` needs an approval
    /// record this command has no way to supply (0A §35).
    ///
    /// `obligation_ref` is matched against the obligation's
    /// `requirement` text, exactly and case-sensitively.
    ///
    /// Deliberately **not** an MCP tool. §102 item 25: "MCP cannot
    /// directly set state, gates, obligations, approvals, or
    /// evidence." A policy verdict is the runtime's to record from an
    /// evaluation, not a field an agent may write.
    RecordObligationOutcome {
        job_id: JobId,
        obligation_ref: String,
        outcome: ObligationOutcome,
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
    /// **No path reaches this from `next_actions`.** At
    /// `ReadyForApproval` the projection emits `allowed: vec![]` and
    /// `requires_human: Some(HumanAction::ApproveRelease)` — approval
    /// is a human's move, so `RequestApproval` is not an
    /// `AllowedAction` and `next_actions` does not name it. An agent
    /// that follows `next_actions` correctly here **stops**, which is
    /// what `SKILL.md` tells it to do.
    ///
    /// The command still exists so the boundary is *observable*: an
    /// orchestrator that reaches for the obvious write anyway gets a
    /// typed refusal naming it, rather than a missing-arms
    /// `Other("unhandled command")` that reads like a bug. That is
    /// its whole job now — not to be the advertised next move, which
    /// an earlier wire format made it.
    ///
    /// > This paragraph previously claimed the opposite, and was right
    /// > to: at 0B.9 (`349c8f1`) `ReadyForApproval` emitted
    /// > `allowed: vec![AllowedAction::RequestApproval]`. 0B.10's C3
    /// > split moved human moves onto `requires_human`, and the
    /// > behaviour changed silently — no test failed, because the
    /// > test that covers this state
    /// > (`mcp_acceptance_scenario.rs`, step 29) asserts the
    /// > *behaviour*, and a doc comment is not behaviour. See D-20.
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
    /// Move a job into `HumanReviewRequired` (0A §21).
    ///
    /// §21 says deterministic orchestration *may* do this from any
    /// nonterminal state, so this is an explicit, deliberate act
    /// rather than a side effect of anything. That is deliberate:
    /// 0B §83 tells the agent to *stop* when the runtime reports
    /// `HumanReviewRequired`, so a transition that fired on every
    /// transient policy failure would strand a job the agent is
    /// supposed to repair and re-run — which is precisely §101's
    /// shape at step 18.
    ///
    /// Entering writes a [`ResumeRecord`](ironmaint_state::ResumeRecord)
    /// naming the state to return to. 0A §21: "Returning ...
    /// requires a recorded event containing the resume state. Do
    /// not infer the previous state from history at runtime.
    /// Record it explicitly." The record is written **here**, at the
    /// moment of entry, from the pre-transition projection — which
    /// is what makes it a record rather than a later inference.
    EnterHumanReview {
        job_id: JobId,
        /// Why a human is needed. Carried into the event trail so
        /// whoever later resumes the job knows what they were
        /// being asked to look at.
        reason: String,
    },
    /// Return a job to the state named by its recorded
    /// [`ResumeRecord`](ironmaint_state::ResumeRecord).
    ///
    /// The target is read back from the event log, never derived
    /// from the job's history: a job in an exceptional state whose
    /// record is missing is refused with
    /// [`RuntimeErrorKind::InvalidInput`](crate::error::RuntimeErrorKind::InvalidInput),
    /// because the alternative — guessing the prior state — is what
    /// 0A §21 forbids.
    ///
    /// Only ever returns the job to where a human-side actor
    /// already chose to put it.
    ResumeJob { job_id: JobId },
    /// Snapshot "everything required to release" for the job's
    /// active candidate (0A §42, §101 step 26).
    ///
    /// The snapshot is bound to whatever candidate is active *now*
    /// and lists the gate and obligation ids that candidate is
    /// held against — so a later candidate capture does not
    /// silently inherit it. Creating one says nothing about
    /// releasability; 0A §42 is explicit that "Release candidate
    /// creation itself does not mean releasable. The state engine
    /// determines whether it may become `ReadyForApproval`."
    ///
    /// This is a read-model assembly, not a state change: it
    /// advances no `JobState`, creates no
    /// [`PrivilegedOperation`](ironmaint_policy::PrivilegedOperation),
    /// and is not reachable over MCP — §101 step 29 asserts the
    /// first two, and §102 item 25 forbids the rest.
    CreateReleaseCandidate { job_id: JobId },
}
