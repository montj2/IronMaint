//! `RuntimeService` — façade that ties the store, the state
//! engine, the executor, and the workspace into a single
//! command/query API.
//!
//! Commands mutate state through the store and the state
//! engine. State changes go through `ironmaint-state::TransitionEngine`,
//! never through ad-hoc writes. Queries are read-only projections
//! and never block a concurrent command.
//!
//! The runtime never touches distribution-specific state. It
//! speaks only in terms of `DistributionFamily`-tagged strings
//! (`DistributionRef` from core), opaque package versions, adapter
//! capabilities.

use std::sync::Arc;

use crate::outcome::CheckOutcome;
use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, DomainEventId, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, SourceCandidate,
};
use ironmaint_evidence::{Evidence, EvidenceProducer, EvidenceScope, GateStatus};
use ironmaint_executor::{ExecutionRequest, Executor, ToolRegistry};
use ironmaint_policy::{ObligationOutcome, ObligationStatus, PrivilegedOperation};
use ironmaint_state::{
    CandidateActivated, JobEvent, ToolOutcome, ToolRunFinished, TransitionBlocker,
    TransitionDecision,
};
use ironmaint_store::CheckDefinition;
use ironmaint_store::IronMaintStore;
use ironmaint_store::envelope::EventEnvelope;

use crate::adapters::AdapterRegistry;
use crate::clock::Clock;
use crate::command::RuntimeCommand;
use crate::error::{RuntimeError, RuntimeErrorKind};
use crate::next_actions::{ActionBlocker, AllowedAction, HumanAction, JobNextActions};
use crate::query::RuntimeQuery;
use crate::reconcile::ReconcileOutcome;

/// Result of handling a command: the new projection's state
/// version, the new event sequence, and the (possibly empty)
/// set of side-effects the daemon should perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub new_version: u64,
    pub new_sequence: u64,
    pub side_effects: Vec<String>,
}

/// Result of handling a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryResult {
    Job(serde_json::Value),
    Projection(serde_json::Value),
    NextActions(JobNextActions),
    CheckOutcome(CheckOutcome),
    Operation(PrivilegedOperation),
    /// The §42 snapshot for a job's active candidate.
    ReleaseCandidate(ironmaint_policy::ReleaseCandidate),
    /// PHASE-1.md §31 — `evidence.list`. A list of `Evidence`
    /// rows for one job (optionally narrowed to one candidate
    /// fingerprint).
    EvidenceList(Vec<ironmaint_evidence::Evidence>),
    /// PHASE-1.md §31 — `evidence.get`. A single `Evidence`
    /// row, looked up by id.
    Evidence(ironmaint_evidence::Evidence),
    /// PHASE-1.md §31 — `evidence.artifact.read`. The bytes
    /// of a `Report` artifact bound to an `Evidence` row,
    /// plus the artifact's declared `media_type`. The bytes
    /// are wrapped in `ArtifactBytes` so the schema can pin
    /// the encoding (base64) at the wire boundary.
    EvidenceArtifact(ArtifactBytes),
}

/// One artifact's bytes as the runtime returns them to a
/// query caller.
///
/// `bytes` is the raw artifact payload (the on-disk
/// content). The MCP transport carries it as base64 inside
/// JSON, so the `MediaType` is what the wire contract
/// promises to a client; the `bytes` field here is the
/// already-decoded form, and a future non-JSON transport
/// (gRPC, raw HTTP) would not need to re-encode.
///
/// `Digest` is the artifact's content-addressed identity,
/// included so a caller can confirm the on-the-wire bytes
/// are the bytes the runtime stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBytes {
    pub digest: ironmaint_core::Digest,
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// Why `TransitionContext::infrastructure_blocked` is hardcoded
/// `false` at both production call sites, and when that stops being
/// the right answer.
///
/// A named constant rather than two bare `false` literals, so that
/// "both sites, never one" is enforced by the compiler rather than by
/// a comment that the next reader has to notice.
///
/// ## What the flag actually is
///
/// A **transition-time veto**, not a job-state entry.
/// `TransitionEngine::evaluate` checks it at step 6
/// (`crates/ironmaint-state/src/engine.rs:299`) and returns
/// `TransitionBlocker::InfrastructureBlocked` immediately, *before*
/// the gate checks at step 7. So it means "do not evaluate this
/// transition at all", not "move the job into
/// `JobState::InfrastructureBlocked`".
///
/// That is what it is for: infrastructure broken **outside any gate**.
/// The store is unreachable, the workspace is gone, the adapter cannot
/// be constructed. In each of those cases there is no gate to be
/// `Blocked` *in*, because no gate can be evaluated at all — so the
/// flag is the only place that failure can be expressed. The whole
/// `InfrastructureBlocked` path is built, wired and tested; nothing
/// sets this boolean in production. `true` appears exactly once in the
/// tree, in `engine_scenarios.rs`.
///
/// ## Why Phase 1 does not need it
///
/// Spec §35 is the Debian read-only slice: repository inspection,
/// Policy retrieval, the Developer's Reference, BTS read-only. Every
/// input is over the network, which makes Phase 1 the first phase
/// where a *required* input can simply be unavailable.
///
/// That failure **does** have a gate. An unreachable source produces
/// `EvidenceStatus::InfrastructureError` on a real check, which
/// aggregates to `GateStatus::Blocked`, which the engine turns into
/// `TransitionBlocker::IncompleteGate(gate_id)`. That names the gate
/// that could not be answered and keeps the reason in
/// `GateResult::evidence` — strictly more actionable than a bare
/// `"infrastructure blocked"`, and it arrives through the path IronClaw
/// already reacts to differently from a failure.
///
/// ## The wiring criterion
///
/// **Wire it when a failure mode exists that has no gate to carry
/// it.** Until then it is not a gap; it is the absence of a caller
/// for a correct mechanism.
///
/// Note that the gate-aggregation defect fixed in this same branch is
/// what makes the alternative safe to rely on. While `combine_status`
/// ranked `Fail` above `Blocked`, an infrastructure error inside a
/// gate was reported as a verdict and this argument would not have
/// held. The criterion is only true because of that fix.
///
/// ## Both sites, never one
///
/// There are exactly two production constructions of
/// `TransitionContext`: `try_transition`, and the `reconcile` rule
/// walk. **A one-site fix would be worse than the current honest
/// `false`**: it would make a manual transition refuse while
/// `job.reconcile` reported the same job as movable. The two callers
/// would disagree about the same job in the same instant, and the
/// agent has no way to tell which surface it is talking to. Setting
/// this constant flips both or neither.
const INFRASTRUCTURE_BLOCKED: bool = false;

pub struct RuntimeService<S: IronMaintStore + ?Sized, E: Executor + ?Sized> {
    store: Arc<S>,
    clock: Arc<dyn Clock>,
    executor: Arc<E>,
    registry: Arc<ToolRegistry>,
    adapters: AdapterRegistry,
    /// Artifact store for binding normalized tool output to
    /// Evidence rows as `Report` artifacts (PHASE-1.md §31).
    /// `None` means the runtime was constructed without a
    /// writable on-disk artifact root; the runtime then records
    /// Evidence rows without an attached artifact, and the
    /// `evidence.artifact.read` MCP tool (1A.4) cannot serve
    /// the missing report. The production daemon always wires
    /// this; tests may omit it.
    artifacts: Option<Arc<ironmaint_artifacts::ArtifactStore>>,
    /// Per-job workspace, used to compute the on-disk path the
    /// runtime hands to tools that read a checked-out source
    /// tree (PHASE-1.md §16, 1C.1). Mirrors
    /// `ironmaint_mcp::McpRuntime::with_workspace`: the
    /// `McpRuntime` already holds the manager for the
    /// `workspace.*` MCP tools, and a `check.run` dispatch
    /// reaches `build_candidate_input` *through* the runtime
    /// service, so the runtime needs the same manager. `None`
    /// in tests that never `check.run` against a tool with
    /// `ToolInputMode::JsonStdin`; the production daemon
    /// always wires it.
    workspace: Option<Arc<ironmaint_workspace::WorkspaceManager<Arc<S>>>>,
}

impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> std::fmt::Debug for RuntimeService<S, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeService").finish()
    }
}

impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> RuntimeService<S, E> {
    #[must_use]
    pub fn new(
        store: Arc<S>,
        clock: Arc<dyn Clock>,
        executor: Arc<E>,
        registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            store,
            clock,
            executor,
            registry,
            adapters: AdapterRegistry::empty(),
            artifacts: None,
            workspace: None,
        }
    }

    /// Attach the on-disk artifact store used to bind
    /// `ResultNormalizer` output to Evidence rows as `Report`
    /// artifacts (PHASE-1.md §31). Builder-style, like
    /// [`Self::with_adapters`], so existing construction sites
    /// (every test) do not need to spell out
    /// `Arc::new(ArtifactStore::open(...))` only to leave it
    /// unused.
    #[must_use]
    pub fn with_artifact_store(
        mut self,
        artifacts: Arc<ironmaint_artifacts::ArtifactStore>,
    ) -> Self {
        self.artifacts = Some(artifacts);
        self
    }

    /// Attach a distribution adapter registry, the PHASE-0B.md
    /// §67 "load adapter registry" startup step.
    ///
    /// A builder rather than a fifth [`new`] argument: the
    /// registry is optional (a runtime with no adapters still
    /// captures candidates, it just derives no checks), and 28
    /// existing construction sites — every one of them a test —
    /// should not have to spell out `AdapterRegistry::empty()`.
    ///
    /// Consuming and returning `Self` lets the daemon chain
    /// `.with_adapters(...)` onto the `Arc::new(...)` it already
    /// builds, without a second mutable binding.
    #[must_use]
    pub fn with_adapters(mut self, adapters: AdapterRegistry) -> Self {
        self.adapters = adapters;
        self
    }

    /// Attach the workspace manager so `build_candidate_input`
    /// (1C.1) can include the on-disk path of the candidate's
    /// working tree in the JSON payload handed to tools that
    /// opt into `ToolInputMode::JsonStdin`. The runtime is the
    /// right home for this: `check.run` reaches
    /// `build_candidate_input` through the service, not through
    /// the `McpRuntime` that already holds the manager for the
    /// `workspace.*` MCP tools.
    ///
    /// Optional because tests that never exercise
    /// `ToolInputMode::JsonStdin` (most of the existing
    /// conformance suite) do not need it; the production
    /// daemon always wires it.
    #[must_use]
    pub fn with_workspace(
        mut self,
        workspace: Arc<ironmaint_workspace::WorkspaceManager<Arc<S>>>,
    ) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// The adapter registry this runtime derives plans from.
    #[must_use]
    pub fn adapters(&self) -> &AdapterRegistry {
        &self.adapters
    }

    pub async fn handle_command(&self, cmd: RuntimeCommand) -> Result<CommandResult, RuntimeError> {
        match cmd {
            RuntimeCommand::CreateJob {
                orchestrator,
                package,
            } => self.handle_create_job(orchestrator, package).await,
            RuntimeCommand::CaptureCandidate { job_id, candidate } => {
                self.handle_capture_candidate(job_id, candidate).await
            }
            RuntimeCommand::MaterializeChecks {
                job_id,
                candidate,
                planned,
            } => {
                self.handle_materialize_checks(job_id, candidate, planned)
                    .await
            }
            RuntimeCommand::SetActiveCandidate {
                job_id,
                fingerprint,
            } => self.handle_set_active_candidate(job_id, fingerprint).await,
            RuntimeCommand::RecordCheckEvidence {
                job_id,
                tool_key,
                evidence_kind,
                status,
                producer,
            } => {
                self.handle_record_check_evidence(job_id, tool_key, evidence_kind, status, producer)
                    .await
            }
            RuntimeCommand::RecordObligationOutcome {
                job_id,
                obligation_ref,
                outcome,
            } => {
                self.handle_record_obligation_outcome(job_id, obligation_ref, outcome)
                    .await
            }
            RuntimeCommand::RunCheck {
                job_id,
                check_id,
                retry_class,
            } => self.handle_run_check(job_id, check_id, retry_class).await,
            RuntimeCommand::RequestApproval { job_id, category } => {
                Err(refuse_approval(job_id, &category))
            }
            RuntimeCommand::Reconcile { job_id } => {
                let outcome = self.reconcile(job_id).await?;
                Ok(CommandResult {
                    new_version: 0,
                    new_sequence: 0,
                    side_effects: vec![format!("reconcile:{outcome:?}")],
                })
            }
            RuntimeCommand::EnterHumanReview { job_id, reason } => {
                self.handle_enter_human_review(job_id, reason).await
            }
            RuntimeCommand::ResumeJob { job_id } => self.handle_resume_job(job_id).await,
            RuntimeCommand::CreateReleaseCandidate { job_id } => {
                self.handle_create_release_candidate(job_id).await
            }
        }
    }

    pub async fn handle_query(&self, query: RuntimeQuery) -> Result<QueryResult, RuntimeError> {
        match query {
            RuntimeQuery::GetJob { job_id } => {
                let projection = self
                    .store
                    .get_projection(job_id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
                Ok(QueryResult::Projection(
                    serde_json::to_value(&projection).map_err(|e| {
                        RuntimeError::new(RuntimeErrorKind::Other(e.to_string()), "serialize")
                    })?,
                ))
            }
            RuntimeQuery::GetProjection { job_id } => {
                let projection = self
                    .store
                    .get_projection(job_id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
                Ok(QueryResult::Projection(
                    serde_json::to_value(&projection).map_err(|e| {
                        RuntimeError::new(RuntimeErrorKind::Other(e.to_string()), "serialize")
                    })?,
                ))
            }
            RuntimeQuery::ListNextActions { job_id } => {
                let projection = self
                    .store
                    .get_projection(job_id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
                let mut actions = project_next_actions(job_id, projection.state);
                self.attach_pending_checks(job_id, &mut actions).await?;
                self.attach_obligation_state(job_id, &mut actions).await?;
                self.attach_resume_action(job_id, &mut actions).await?;
                Ok(QueryResult::NextActions(actions))
            }
            RuntimeQuery::GetCheckOutcome { check_id } => self.get_check_outcome(check_id).await,
            RuntimeQuery::GetOperation { operation_id } => {
                let operation =
                    self.store
                        .get_operation(operation_id)
                        .await
                        .map_err(|e| match e.kind() {
                            ironmaint_store::StoreErrorKind::NotFound => RuntimeError::new(
                                RuntimeErrorKind::InvalidInput,
                                format!("unknown operation {operation_id}"),
                            ),
                            _ => RuntimeError::new(RuntimeErrorKind::Store, e.to_string()),
                        })?;
                Ok(QueryResult::Operation(operation))
            }
            RuntimeQuery::GetReleaseCandidate { job_id } => {
                self.get_release_candidate_for_job(job_id).await
            }
            // PHASE-1.md §31 — `evidence.list`. The
            // job_id is required so a caller can never
            // enumerate evidence across jobs without
            // explicitly naming them; the optional
            // `candidate_fingerprint` is a second scope
            // check (filter, not widen).
            RuntimeQuery::ListEvidence {
                job_id,
                candidate_fingerprint,
            } => {
                let mut rows = self
                    .store
                    .list_evidence_for_job(job_id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
                if let Some(fp) = candidate_fingerprint {
                    rows.retain(|e| e.candidate == fp);
                }
                Ok(QueryResult::EvidenceList(rows))
            }
            // PHASE-1.md §31 — `evidence.get`. A
            // `StoreErrorKind::NotFound` is mapped to
            // `InvalidInput` so the MCP layer returns a
            // typed "no such evidence id" error to the
            // caller; everything else is a `Store`
            // error.
            RuntimeQuery::GetEvidence { evidence_id } => {
                let evidence =
                    self.store
                        .get_evidence(evidence_id)
                        .await
                        .map_err(|e| match e.kind() {
                            ironmaint_store::StoreErrorKind::NotFound => RuntimeError::new(
                                RuntimeErrorKind::InvalidInput,
                                format!("unknown evidence id {evidence_id}"),
                            ),
                            _ => RuntimeError::new(RuntimeErrorKind::Store, e.to_string()),
                        })?;
                Ok(QueryResult::Evidence(evidence))
            }
            // PHASE-1.md §31 — `evidence.artifact.read`.
            // Look the row up, find the matching
            // `ArtifactRef` (refuse a wrong `artifact_id`
            // with `InvalidInput`), refuse if the runtime
            // has no `ArtifactStore` configured, then
            // call `store.get(&digest)` and return the
            // bytes wrapped in `QueryResult::EvidenceArtifact`.
            RuntimeQuery::ReadEvidenceArtifact {
                evidence_id,
                artifact_id,
            } => {
                let evidence =
                    self.store
                        .get_evidence(evidence_id)
                        .await
                        .map_err(|e| match e.kind() {
                            ironmaint_store::StoreErrorKind::NotFound => RuntimeError::new(
                                RuntimeErrorKind::InvalidInput,
                                format!("unknown evidence id {evidence_id}"),
                            ),
                            _ => RuntimeError::new(RuntimeErrorKind::Store, e.to_string()),
                        })?;
                let artifact_ref = evidence
                    .artifacts
                    .iter()
                    .find(|a| a.id == artifact_id)
                    .ok_or_else(|| {
                        RuntimeError::new(
                            RuntimeErrorKind::InvalidInput,
                            format!("evidence {evidence_id} has no artifact with id {artifact_id}"),
                        )
                    })?;
                let store = self.artifacts.as_ref().ok_or_else(|| {
                    RuntimeError::new(
                        RuntimeErrorKind::InvalidInput,
                        "evidence.artifact.read: runtime has no artifact store configured"
                            .to_string(),
                    )
                })?;
                // The artifact store's `get` takes
                // `&Sha256Hex`, not `&Digest`. Convert
                // the typed `Digest` to its hex form
                // and validate it as a `Sha256Hex`.
                // `Digest::algorithm` is `Sha256` here
                // by construction (the runtime only
                // stores `Report` artifacts under
                // SHA-256), so any other algorithm
                // would be a store invariant violation.
                let hex = artifact_ref.digest.value.clone();
                let sha256_hex =
                    ironmaint_artifacts::hash::Sha256Hex::from_hex(&hex).ok_or_else(|| {
                        RuntimeError::new(
                            RuntimeErrorKind::Store,
                            format!("artifact digest {hex} is not a valid SHA-256 hex string"),
                        )
                    })?;
                let bytes = store
                    .get(&sha256_hex)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
                Ok(QueryResult::EvidenceArtifact(ArtifactBytes {
                    digest: artifact_ref.digest.clone(),
                    media_type: artifact_ref.media_type.clone(),
                    bytes,
                }))
            }
        }
    }

    /// The snapshot assembled for the job's active candidate.
    ///
    /// Two errors rather than an empty result, because "there is no
    /// active candidate" and "the active candidate has no snapshot
    /// yet" are different facts with different next moves — one is
    /// mid-workflow, the other is at the exit with the assembly
    /// outstanding. `Option` would flatten them.
    async fn get_release_candidate_for_job(
        &self,
        job_id: JobId,
    ) -> Result<QueryResult, RuntimeError> {
        let Some(fingerprint) = self.active_fingerprint(job_id).await? else {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("job {job_id} has no active candidate, so no release candidate"),
            ));
        };
        let snapshots = self
            .store
            .list_release_candidates_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        snapshots
            .into_iter()
            .find(|r| r.source == fingerprint)
            .map(QueryResult::ReleaseCandidate)
            .ok_or_else(|| {
                RuntimeError::new(
                    RuntimeErrorKind::InvalidInput,
                    format!(
                        "job {job_id} has no release candidate for its active candidate \
                         {fingerprint}; run RuntimeCommand::CreateReleaseCandidate first"
                    ),
                )
            })
    }

    /// The active candidate's fingerprint, read through the
    /// projection and then the candidate row.
    ///
    /// Gates, obligations, and evidence are all keyed by
    /// fingerprint rather than by job or candidate id, so this
    /// three-line walk is the precondition for reading any of them.
    /// Three call sites need it; a fourth would be a sign the store
    /// should offer the join directly.
    async fn active_fingerprint(
        &self,
        job_id: JobId,
    ) -> Result<Option<ironmaint_core::CandidateFingerprint>, RuntimeError> {
        let projection = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let Some(candidate_id) = projection.active_candidate else {
            return Ok(None);
        };
        let source = self
            .store
            .get_source_candidate(candidate_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        Ok(Some(source.fingerprint().clone()))
    }

    /// Replace the pure projection's unconditional
    /// `SatisfyObligation` with the actual outstanding obligations.
    ///
    /// `project_next_actions` is pure, so at `ReleaseReview` and
    /// `FinalValidation` it can only say "policy completion is a
    /// person's judgement" without reading whether any obligation is
    /// outstanding. That was defensible while nothing could ever make
    /// one fail. It is not now: with `RecordObligationOutcome` a
    /// mandatory obligation can genuinely be in `Fail`, and a caller
    /// told only "requires_human: satisfy_obligation" would be told
    /// nothing about *which* assertion failed or that it failed at
    /// all. `ActionBlocker::ObligationPending` exists to carry that
    /// and, before this, was never constructed.
    ///
    /// Only mandatory + applicable obligations are reported, matching
    /// `check_obligations` in the engine exactly: a `Recommended`
    /// obligation that has not been evaluated does not block, so
    /// calling it a blocker would send an agent after work the state
    /// machine does not require.
    async fn attach_obligation_state(
        &self,
        job_id: JobId,
        actions: &mut crate::next_actions::JobNextActions,
    ) -> Result<(), RuntimeError> {
        // Whether a *person* has to act is state-dependent, and the
        // pure projection already decided it: policy completion is
        // someone's judgement only at `ReleaseReview` and
        // `FinalValidation`. Whether an obligation is *outstanding*
        // is not state-dependent, and until now this function only
        // ran when the pure projection had already said
        // `SatisfyObligation` — so at every earlier state a mandatory
        // obligation that had come back `Fail` was invisible. An
        // agent at `EventDetected` whose policy check failed was
        // told its blockers were empty and its only move was to run
        // the check again, which produces the same evidence and the
        // same verdict. §83's loop has no way to express "the
        // assertion is not met, and only different source will meet
        // it"; SKILL.md has to tell the agent to patch and re-run
        // instead, because the tool surface never said so.
        let human_decision = matches!(
            actions.requires_human,
            Some(HumanAction::SatisfyObligation { .. })
        );

        // No active candidate means no obligation is bound to
        // anything, so there is nothing to report. A fresh job's
        // `next_actions` must not fail.
        let Some(fingerprint) = self.active_fingerprint(job_id).await? else {
            return Ok(());
        };

        let ids = self
            .store
            .list_obligations_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        for id in ids {
            let ob = self
                .store
                .get_obligation(id)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            if ob.candidate != fingerprint
                || ob.strength != ironmaint_policy::ObligationStrength::Mandatory
                || ob.applicability != ironmaint_policy::Applicability::Applicable
            {
                continue;
            }
            // `NotEvaluated` is not an outstanding assertion, it is
            // an unasked question, and the question is already
            // represented by the pending `RunCheck`. Reporting it
            // would tell an agent to go satisfy an obligation whose
            // verdict does not exist yet.
            if !matches!(
                ob.status,
                ObligationStatus::Fail | ObligationStatus::RequiresReview
            ) {
                continue;
            }
            actions.blockers.push(ActionBlocker::ObligationPending {
                reference: ob.requirement.clone(),
            });
            if human_decision {
                // The first outstanding mandatory obligation is the
                // one to work on; naming it is the difference
                // between an agent that can act and one that has to
                // guess.
                actions.requires_human = Some(HumanAction::SatisfyObligation {
                    reference: Some(ob.requirement.clone()),
                });
            }
            break;
        }
        Ok(())
    }

    /// Offer [`AllowedAction::ResumeJob`] when — and only when —
    /// the job is in an exceptional state *and* has a recorded
    /// resume state to return to (0A §21).
    ///
    /// The exceptional arms of the pure projection advertise
    /// nothing, which reads as a dead end: an agent that lands in
    /// `HumanReviewRequired` has no move, and no blocker that would
    /// ever clear either. This is the same "wait for a blocker
    /// that will never arrive" trap `SKILL.md` already warns about
    /// for approvals.
    ///
    /// The existence check is a log read, which is why the
    /// attachment lives here rather than in the pure projection.
    /// Gating on it is not pedantry: a job in an exceptional state
    /// with no record cannot be resumed at all, so advertising the
    /// action would send an agent to a call that is guaranteed to
    /// fail.
    async fn attach_resume_action(
        &self,
        job_id: JobId,
        actions: &mut crate::next_actions::JobNextActions,
    ) -> Result<(), RuntimeError> {
        let projection = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        if !projection.state.is_exceptional() {
            return Ok(());
        }
        if self
            .load_resume_record(job_id, projection.state)
            .await?
            .is_some()
        {
            actions
                .allowed
                .push(crate::next_actions::AllowedAction::ResumeJob);
        }
        Ok(())
    }

    /// Resolve a pending gate against the job's materialised
    /// checks: name the tool that would clear it, and surface the
    /// `check_id` an orchestrator needs to run it.
    ///
    /// Without this, `next_actions` could only say "a gate is
    /// pending" and `check.run` was unreachable — nothing else in
    /// the MCP surface hands an agent a `check_id`. The mapping
    /// from tool to `check_id` is a store read, which is why it
    /// lives here rather than in the pure `project_next_actions`.
    async fn attach_pending_checks(
        &self,
        job_id: JobId,
        actions: &mut JobNextActions,
    ) -> Result<(), RuntimeError> {
        // Which checks an agent may run is a fact about the store,
        // not about what the state machine currently wants. A
        // materialised check is runnable the moment it is bound to
        // the active candidate, and §101 depends on that: it runs a
        // build, reads the failure, patches, captures a new
        // candidate, and runs again — all before the job advances a
        // single state.
        //
        // Gating this on `GatePending` (as it used to) meant a job
        // at `EventDetected` advertised no runnable check at all,
        // even though capture had just materialised four.
        // …but only while the job is still repairable. At
        // `ReadyForApproval` the evidence is what a human is about
        // to decide on, and re-running a check would overwrite it;
        // in an exceptional state the job is waiting on a person,
        // and `allowed` already names the one move that is theirs.
        let state = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
            .state;
        if !state.is_repairable() {
            return Ok(());
        }

        let check_ids = self
            .store
            .list_checks_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        if check_ids.is_empty() {
            // No adapter has materialised checks for this job, so
            // there is genuinely no tool to name. Leaving
            // `tool_key` as `None` is the honest answer; the
            // blocker itself still explains the hold.
            return Ok(());
        }

        // Only the active candidate's checks are runnable:
        // `handle_run_check` rejects a stale fingerprint (§30
        // evidence freshness), and advertising an action the
        // runtime will refuse is worse than advertising nothing.
        let active = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
            .active_candidate;
        let active = match active {
            Some(id) => self
                .store
                .get_source_candidate(id)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
                .fingerprint()
                .clone(),
            // No active candidate: nothing is runnable yet, and the
            // `CaptureCandidate` action is already on the list.
            None => return Ok(()),
        };

        let mut first_key: Option<String> = None;
        for check_id in check_ids {
            let Ok(check) = self.store.get_check(check_id).await else {
                continue;
            };
            if check.candidate != active {
                continue;
            }
            let key = check.capability.as_str().to_string();
            if first_key.is_none() {
                first_key = Some(key);
            }
            if !actions
                .allowed
                .contains(&AllowedAction::RunCheck { check_id })
            {
                actions.allowed.push(AllowedAction::RunCheck { check_id });
            }
        }

        for blocker in &mut actions.blockers {
            if let ActionBlocker::GatePending { tool_key } = blocker {
                *tool_key = first_key.clone();
            }
        }
        Ok(())
    }

    /// Read the durable result of a check: the latest evidence
    /// row matching its `evidence_kind`, plus the gate verdict that
    /// row contributed to.
    ///
    /// A check with no evidence yet is not an error — the gate is
    /// simply `NotEvaluated`, which is the state `check.run` leaves
    /// a workflow in before the orchestrator runs anything. That
    /// distinction matters: reporting it as a failure would make
    /// "not run yet" and "ran and failed" indistinguishable.
    async fn get_check_outcome(
        &self,
        check_id: ironmaint_core::CheckId,
    ) -> Result<QueryResult, RuntimeError> {
        let check = self
            .store
            .get_check(check_id)
            .await
            .map_err(|e| match e.kind() {
                ironmaint_store::StoreErrorKind::NotFound => RuntimeError::new(
                    RuntimeErrorKind::InvalidInput,
                    format!("unknown check {check_id}"),
                ),
                _ => RuntimeError::new(RuntimeErrorKind::Store, e.to_string()),
            })?;
        let fingerprint = check.candidate.clone();
        // `GateStore` keys results by `(gate_id, candidate)` and a
        // row is only written once something has been evaluated, so
        // `NotFound` is the normal state of a check that has not
        // run — not a store failure. Turning it into an error here
        // would make the first `check.run` of a job impossible to
        // observe and would report "not run yet" as an
        // infrastructure problem.
        let gate_status = match self
            .store
            .get_gate_result(check.gate_id, &fingerprint)
            .await
        {
            Ok(gate) => gate.status,
            Err(e) if e.kind == ironmaint_store::StoreErrorKind::NotFound => {
                GateStatus::NotEvaluated
            }
            Err(e) => return Err(RuntimeError::new(RuntimeErrorKind::Store, e.to_string())),
        };
        let evidence = crate::check::latest_evidence_for_check(&*self.store, &fingerprint, &check)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let (evidence_id, evidence_status, truncated) = match evidence {
            Some(ev) => (Some(ev.id), Some(ev.status), ev.truncated),
            None => (None, None, false),
        };
        Ok(QueryResult::CheckOutcome(CheckOutcome {
            job_id: check.job_id,
            check_id,
            gate_id: check.gate_id,
            candidate: fingerprint,
            tool_key: check.capability.as_str().to_string(),
            evidence_id,
            evidence_status,
            gate_status,
            truncated,
        }))
    }
}

// ---------------------------------------------------------------------------
// Command handlers.
//
// Each handler is a private `async fn` so the dispatcher above stays
// readable. They share the small "append a domain event" helper below.
// ---------------------------------------------------------------------------

impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> RuntimeService<S, E> {
    /// `CreateJob`: mint a fresh [`MaintenanceJob`] + [`JobProjection`],
    /// persist it at `version = 0`, and seed the event log with a
    /// `JobEvent::JobCreated` carrying that projection, followed by a
    /// `JobEvent::Domain` referencing the initiating event id.
    async fn handle_create_job(
        &self,
        orchestrator: crate::orchestrator::OrchestratorRef,
        package: ironmaint_core::PackageIdentity,
    ) -> Result<CommandResult, RuntimeError> {
        let now = self.clock.now_utc();
        let job_id = JobId::new();
        let initiating_event = MaintenanceEventId::new();

        let job = MaintenanceJob::new(job_id, package, initiating_event, now);
        let projection = JobProjection {
            job,
            state: JobState::EventDetected,
            active_candidate: None,
            version: 0,
            updated_at: now,
        };

        // First projection write requires expected_version = 0
        // (MockStore / SQLite both reject mismatches).
        self.store
            .put_projection(&projection, 0)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // Seed the event log. Two events, in this order:
        //
        // 1. `JobCreated` carries the projection as it looked at
        //    birth. It must be first: it is the seed a replay starts
        //    from, and without it a log whose only other early event
        //    is a bare `Domain` reference carries no projection at
        //    all — which is what made `rebuild_projection` fail for
        //    every job the runtime had ever created (D-14).
        // 2. `Domain` points at the event id that initiated the job,
        //    the audit reference 0A §30 asks for.
        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let created = EventEnvelope::new(
            MaintenanceEventId::from_uuid(initiating_event.as_uuid()),
            job_id,
            sequence,
            now,
            JobEvent::JobCreated(projection.clone()),
        );
        self.store
            .append_event(&created)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let domain_event_id = DomainEventId::new();
        let envelope = EventEnvelope::new(
            MaintenanceEventId::from_uuid(domain_event_id.as_uuid()),
            job_id,
            sequence,
            now,
            JobEvent::Domain(domain_event_id),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let side_effect = format!(
            "job:{job_id} created by orchestrator={:?}",
            orchestrator.kind
        );

        Ok(CommandResult {
            new_version: 0,
            new_sequence: sequence,
            side_effects: vec![side_effect],
        })
    }

    /// Evaluate and apply a state transition through
    /// `TransitionEngine` (§18). The runtime is the only caller
    /// of `engine.evaluate`; agents and adapters cannot mutate
    /// `JobState`.
    ///
    /// Steps:
    ///  1. Load the projection. If the projection's `version`
    ///     doesn't match `expected_version`, return
    ///     `RuntimeErrorKind::ConcurrentModification` (the engine
    ///     will return the same decision; we surface it as a
    ///     well-typed runtime error instead of a generic blocker).
    ///  2. Build the [`TransitionContext`] from the loaded state
    ///     (gates, obligations, candidate fingerprint, etc.).
    ///  3. Call `engine.apply` to produce the
    ///     `StateTransitioned` event. On `StaleProjection`,
    ///     surface `ConcurrentModification`; on `Blocked`
    ///     return the blocker chain as the error message.
    ///  4. Persist the new projection at
    ///     `expected_version + 1` and append the audit event.
    ///
    /// # Errors
    ///
    /// - `RuntimeErrorKind::ConcurrentModification` on stale
    ///   version.
    /// - `RuntimeErrorKind::Store` on persistence failure.
    /// - `RuntimeErrorKind::InvalidInput` for any other blocker
    ///   (the engine returns `Blocked(Vec<TransitionBlocker>)`
    ///   with the precise reasons).
    pub async fn try_transition(
        &self,
        job_id: JobId,
        target: ironmaint_core::JobState,
        expected_version: u64,
    ) -> Result<ironmaint_state::Transition, RuntimeError> {
        self.try_transition_with_resume(job_id, target, expected_version, None)
            .await
    }

    /// [`Self::try_transition`] with the [`ironmaint_state::ResumeRecord`]
    /// that 0A §21 requires in order to leave an exceptional state.
    ///
    /// The public `try_transition` keeps its three-argument shape
    /// and passes `None`, because every other caller is either
    /// walking the forward rule table (where a resume is
    /// meaningless) or driving a test. Only [`Self::handle_resume_job`]
    /// passes a record, and it reads that record back out of the
    /// event log rather than constructing one — which is the whole
    /// point of §21: "Do not infer the previous state from history
    /// at runtime. Record it explicitly."
    async fn try_transition_with_resume(
        &self,
        job_id: JobId,
        target: ironmaint_core::JobState,
        expected_version: u64,
        resume: Option<&ironmaint_state::ResumeRecord>,
    ) -> Result<ironmaint_state::Transition, RuntimeError> {
        use ironmaint_core::GateId;
        use ironmaint_state::{TransitionContext, TransitionEngine, TransitionRequest};
        use std::collections::BTreeMap;

        let projection = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        if projection.version != expected_version {
            return Err(RuntimeError::new(
                RuntimeErrorKind::ConcurrentModification,
                format!(
                    "expected version {expected_version}, found {}",
                    projection.version
                ),
            ));
        }

        // Resolve the active candidate fingerprint (None in
        // pre-capture states).
        let active_candidate_fingerprint: Option<ironmaint_core::CandidateFingerprint> =
            match projection.active_candidate {
                Some(cid) => {
                    let source =
                        self.store.get_source_candidate(cid).await.map_err(|e| {
                            RuntimeError::new(RuntimeErrorKind::Store, e.to_string())
                        })?;
                    Some(source.fingerprint().clone())
                }
                None => None,
            };

        // Gate definitions + per-candidate GateResults.
        let gate_ids = self
            .store
            .list_gates_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let mut gates: BTreeMap<GateId, ironmaint_evidence::GateResult> = BTreeMap::new();
        let mut gate_definitions: BTreeMap<GateId, ironmaint_evidence::GateDefinition> =
            BTreeMap::new();
        for gid in &gate_ids {
            let def = self
                .store
                .get_gate_definition(*gid)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            gate_definitions.insert(*gid, def);
            if let Some(fp) = active_candidate_fingerprint.as_ref()
                && let Ok(r) = self.store.get_gate_result(*gid, fp).await
            {
                gates.insert(*gid, r);
            }
        }

        // Obligations for the active candidate.
        let obligations = {
            let mut set = ironmaint_policy::ObligationSet::new();
            let all = self
                .store
                .list_obligations_for_job(job_id)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            for id in all {
                let ob = self
                    .store
                    .get_obligation(id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
                if let Some(fp) = active_candidate_fingerprint.as_ref() {
                    if ob.candidate != *fp {
                        continue;
                    }
                } else {
                    continue;
                }
                set.insert(ob);
            }
            set
        };

        let approvals: BTreeMap<ironmaint_core::ApprovalId, ironmaint_policy::ApprovalDecision> =
            BTreeMap::new();
        let approval_requirements: BTreeMap<
            ironmaint_core::ApprovalId,
            ironmaint_policy::ApprovalRequirement,
        > = BTreeMap::new();

        let now = self.clock.now_utc();
        let request = TransitionRequest {
            job_id,
            expected_version,
            target,
            now,
        };
        let ctx = TransitionContext {
            current: &projection,
            active_candidate_fingerprint: active_candidate_fingerprint.clone(),
            gates: &gates,
            gate_definitions: &gate_definitions,
            approval_requirements: &approval_requirements,
            obligations: &obligations,
            approvals: &approvals,
            // A decision, not an oversight. See `INFRASTRUCTURE_BLOCKED`.
            infrastructure_blocked: INFRASTRUCTURE_BLOCKED,
            resume_event: resume,
        };

        let engine = TransitionEngine::new();
        let applied = match engine.apply(request, &ctx) {
            Ok(s) => s,
            Err(ironmaint_state::TransitionApplyError::StaleProjection { expected, found }) => {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::ConcurrentModification,
                    format!("expected {expected}, found {found}"),
                ));
            }
            Err(ironmaint_state::TransitionApplyError::Blocked(blockers)) => {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::InvalidInput,
                    format_blockers(&blockers),
                ));
            }
        };

        // The applied transition produced a new projection. Persist
        // it at the expected+1 version slot, mapping
        // StaleProjection to ConcurrentModification.
        let new_projection = ironmaint_core::JobProjection {
            job: projection.job.clone(),
            state: applied.transition.to,
            active_candidate: projection.active_candidate,
            version: expected_version + 1,
            updated_at: applied.occurred_at,
        };
        self.store
            .put_projection(&new_projection, expected_version)
            .await
            .map_err(|e| {
                if *e.kind() == ironmaint_store::StoreErrorKind::Conflict
                    || *e.kind() == ironmaint_store::StoreErrorKind::SequenceOutOfRange
                {
                    RuntimeError::new(RuntimeErrorKind::ConcurrentModification, e.to_string())
                } else {
                    RuntimeError::new(RuntimeErrorKind::Store, e.to_string())
                }
            })?;

        // Append the audit event.
        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let envelope = ironmaint_store::envelope::EventEnvelope::new(
            ironmaint_core::MaintenanceEventId::new(),
            job_id,
            sequence,
            now,
            ironmaint_state::JobEvent::Transitioned(applied),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        Ok(ironmaint_state::Transition {
            from: ctx.current.state,
            to: new_projection.state,
            rule_index: 0, // rule_index is carried inside `applied.transition`; surfaced via the audit event.
        })
    }

    /// The resume state recorded for a job currently sitting in
    /// `from`, if any.
    ///
    /// Reads the last matching [`JobEvent::ResumeRecorded`] out of
    /// the log rather than reconstructing one. 0A §21: "Do not
    /// infer the previous state from history at runtime. Record it
    /// explicitly." The distinction is the whole feature — a
    /// derived answer would be indistinguishable from a recorded
    /// one right up until the day the job took a different path
    /// through the exceptional state than the reconstruction
    /// assumes.
    ///
    /// Taking the *last* match rather than the first is what makes
    /// repeated excursions work: a job that is reviewed, resumed,
    /// and reviewed again carries two records, and the current one
    /// is the second.
    async fn load_resume_record(
        &self,
        job_id: JobId,
        from: ironmaint_core::JobState,
    ) -> Result<Option<ironmaint_state::ResumeRecord>, RuntimeError> {
        let events = self
            .store
            .list_events_for_job(job_id, 1, None)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        Ok(events.iter().rev().find_map(|env| match &env.event {
            ironmaint_state::JobEvent::ResumeRecorded(r) if r.from == from => Some(r.clone()),
            _ => None,
        }))
    }

    /// `EnterHumanReview`: move the job to `HumanReviewRequired`
    /// and record where it should return to (0A §21).
    ///
    /// The engine permits this from any nonterminal state with no
    /// requirements, so there is nothing to evaluate — the work is
    /// in writing the record, and in doing it *before* the state
    /// moves so the pre-transition state is the one captured.
    async fn handle_enter_human_review(
        &self,
        job_id: JobId,
        reason: String,
    ) -> Result<CommandResult, RuntimeError> {
        let projection = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // Re-entering an exceptional state is refused rather than
        // allowed. The engine would permit `HumanReviewRequired →
        // HumanReviewRequired` — it is "any nonterminal state" —
        // but doing so would overwrite the recorded resume target
        // with `HumanReviewRequired` itself, leaving a job that can
        // never leave. That is a hole the engine cannot see and
        // this guard closes.
        if projection.state.is_exceptional() {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "job {job_id} is already in {}; resume it instead of re-entering, \
                     so the recorded resume state is not overwritten",
                    projection.state
                ),
            ));
        }

        // Captured before the transition: after it, `projection.state`
        // is stale and naming the wrong thing here would record a
        // resume target of `HumanReviewRequired`.
        let resume_to = projection.state;
        let now = self.clock.now_utc();

        self.try_transition_with_resume(
            job_id,
            ironmaint_core::JobState::HumanReviewRequired,
            projection.version,
            None,
        )
        .await?;

        let record = ironmaint_state::ResumeRecord::new(
            ironmaint_core::JobState::HumanReviewRequired,
            resume_to,
            now,
        );
        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let envelope = ironmaint_store::envelope::EventEnvelope::new(
            ironmaint_core::MaintenanceEventId::new(),
            job_id,
            sequence,
            now,
            ironmaint_state::JobEvent::ResumeRecorded(record.clone()),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        Ok(CommandResult {
            new_version: projection.version + 1,
            new_sequence: sequence,
            side_effects: vec![
                format!("job:{job_id} entered HumanReviewRequired: {reason}"),
                format!("resume state recorded: HumanReviewRequired -> {resume_to}"),
            ],
        })
    }

    /// `ResumeJob`: return the job to the state named by its
    /// recorded resume state (0A §21).
    async fn handle_resume_job(&self, job_id: JobId) -> Result<CommandResult, RuntimeError> {
        let projection = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        if !projection.state.is_exceptional() {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "job {job_id} is in {}, which is not an exceptional state; \
                     only HumanReviewRequired and InfrastructureBlocked can be resumed",
                    projection.state
                ),
            ));
        }

        let record = self
            .load_resume_record(job_id, projection.state)
            .await?
            .ok_or_else(|| {
                RuntimeError::new(
                    RuntimeErrorKind::InvalidInput,
                    format!(
                        "no recorded resume state for job {job_id} in {}; \
                         0A §21 requires the resume state to be recorded at \
                         entry and not inferred from history, so the job \
                         cannot be resumed",
                        projection.state
                    ),
                )
            })?;

        let resume_to = record.to;
        self.try_transition_with_resume(job_id, resume_to, projection.version, Some(&record))
            .await?;

        Ok(CommandResult {
            new_version: projection.version + 1,
            new_sequence: 0,
            side_effects: vec![format!(
                "job:{job_id} resumed from {} to {resume_to}",
                projection.state
            )],
        })
    }

    /// `CreateReleaseCandidate`: assemble the §42 snapshot for the
    /// job's active candidate.
    ///
    /// The baseline's authorities are read off the candidate's own
    /// obligations rather than re-derived from the adapter, because
    /// the obligations are the record of which authorities this
    /// candidate is actually held against — re-deriving would
    /// produce a baseline describing a policy query nobody made.
    ///
    /// A job with no active candidate is refused rather than given
    /// a release candidate bound to nothing: 0A §42 makes
    /// `source` the field that gives the snapshot its meaning.
    async fn handle_create_release_candidate(
        &self,
        job_id: JobId,
    ) -> Result<CommandResult, RuntimeError> {
        let projection = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let Some(candidate_id) = projection.active_candidate else {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("job {job_id} has no active candidate to release"),
            ));
        };
        let candidate = self
            .store
            .get_source_candidate(candidate_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let fingerprint = candidate.fingerprint().clone();

        // Idempotent on (job, fingerprint). A snapshot's entire
        // content is derived from the gates and obligations *of that
        // fingerprint* — both of which are fixed the moment the
        // candidate is captured, because re-running a check writes
        // evidence and a verdict, never a new gate or obligation.
        // So the second call has nothing to add, and without this
        // check it would mint a second, byte-identical snapshot
        // under a fresh id — and then a caller with only a `JobId`
        // would have two correct answers and no way to choose.
        //
        // Returning the existing one also keeps the command honest
        // for the agent that calls it twice because it did not see
        // the first call land.
        for existing in self
            .store
            .list_release_candidates_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
        {
            if existing.source == fingerprint {
                let id = existing.id;
                let gates = existing.gate_ids.len();
                let obligations = existing.obligation_ids.len();
                return Ok(CommandResult {
                    new_version: 0,
                    new_sequence: 0,
                    side_effects: vec![format!(
                        "release_candidate:{id} already assembled for {job_id} at \
                         {fingerprint} ({gates} gate(s), {obligations} obligation(s))"
                    )],
                });
            }
        }

        // Gates: every definition minted for *this* candidate, in
        // any order the store returns them. Filtered by
        // fingerprint rather than by job so a superseded
        // candidate's gates cannot leak into the snapshot — the
        // same candidate-binding rule §30 puts on evidence.
        let mut gate_ids = Vec::new();
        for gate_id in self
            .store
            .list_gates_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
        {
            let definition = self
                .store
                .get_gate_definition(gate_id)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            if definition.candidate == fingerprint {
                gate_ids.push(gate_id);
            }
        }

        let mut authorities: Vec<ironmaint_core::AuthorityId> = Vec::new();
        let mut obligation_ids = Vec::new();
        for obligation_id in self
            .store
            .list_obligations_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
        {
            let obligation = match self.store.get_obligation(obligation_id).await {
                Ok(o) => o,
                // A dangling id is a store inconsistency, not a
                // reason to refuse a release candidate over one
                // unrelated obligation.
                Err(_) => continue,
            };
            if obligation.candidate != fingerprint {
                continue;
            }
            obligation_ids.push(obligation_id);
            let authority = obligation.reference.authority;
            if !authorities.contains(&authority) {
                authorities.push(authority);
            }
        }

        let baseline =
            ironmaint_policy::PolicyBaseline::new(projection.job.package.distribution.clone())
                .with_authorities(authorities);
        let release = ironmaint_policy::ReleaseCandidate::new(
            job_id,
            fingerprint.clone(),
            baseline,
            self.clock.now_utc(),
        );
        let mut release = release;
        for gate_id in gate_ids.iter().copied() {
            release = release.with_gate(gate_id);
        }
        for obligation_id in obligation_ids.iter().copied() {
            release = release.with_obligation(obligation_id);
        }
        let release_id = self
            .store
            .put_release_candidate(&release)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        Ok(CommandResult {
            new_version: 0,
            new_sequence: 0,
            side_effects: vec![format!(
                "release_candidate:{release_id} assembled for {job_id} at {fingerprint} \
                 ({} gate(s), {} obligation(s))",
                release.gate_ids.len(),
                release.obligation_ids.len()
            )],
        })
    }

    /// `reconcile`: walk the static rule table for the job's
    /// current state and advance the projection through every
    /// rule whose requirements are satisfied by the evidence
    /// ledger, gate definitions, and obligation set (§41).
    ///
    /// Stops on:
    /// - terminal state (`Published`/`Cancelled`) — `NoOp`
    /// - exceptional state (`HumanReviewRequired`/`InfrastructureBlocked`) — `Exceptional`
    /// - actor-required transition — `NeedsActorDecision`
    /// - concurrent-modification contention — `ConcurrentModification`
    /// - bounded loop overflow — the last transition it applied
    ///   (defensive; the rule table is acyclic, so unreachable)
    pub async fn reconcile(&self, job_id: JobId) -> Result<ReconcileOutcome, RuntimeError> {
        // The walk is bounded by the rule count: every rule moves
        // a job forward along §20's path, so the loop terminates
        // well inside this bound, and `rules + 1` iterations
        // guarantees the last iteration is the one that *reports*
        // where the walk stopped rather than being consumed by a
        // transition.
        let mut last_advance: Option<ReconcileOutcome> = None;
        for _ in 0..=ironmaint_state::TRANSITION_RULES.len() {
            let projection = self
                .store
                .get_projection(job_id)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, format!("{e}")))?;
            let current = projection.state;
            // Terminal / exceptional short-circuits.
            match current {
                JobState::Published | JobState::Cancelled => {
                    return Ok(ReconcileOutcome::NoOp { current });
                }
                JobState::HumanReviewRequired | JobState::InfrastructureBlocked => {
                    return Ok(ReconcileOutcome::Exceptional { current });
                }
                _ => {}
            }
            // Find the next rule in TRANSITION_RULES where
            // `rule.from == current`. If multiple rules share
            // the same `from`, the first one whose
            // `evaluate` returns `Allowed` wins.
            let engine = ironmaint_state::TransitionEngine::new();
            let mut allowed_candidate: Option<(JobState, usize)> = None;
            let mut blocked_candidate: Option<(JobState, Vec<TransitionBlocker>)> = None;
            let mut needs_actor_candidate: Option<(JobState, Vec<TransitionBlocker>)> = None;
            // We need a `TransitionContext`; we rebuild it once per
            // loop iteration because the projection version may
            // have changed underneath us. Inlining the build
            // mirrors `try_transition`.
            let active_fp: Option<CandidateFingerprint> = match projection.active_candidate {
                Some(cid) => match self.store.get_source_candidate(cid).await {
                    Ok(c) => Some(c.fingerprint().clone()),
                    // Active candidate id is set but the
                    // underlying row is missing — this happens
                    // when concurrent writers have left the
                    // projection in an inconsistent state. The
                    // safe behaviour is to skip the rule walk
                    // and report `NoOp`; the caller can reload
                    // and reattempt.
                    Err(_) => {
                        return Ok(ReconcileOutcome::NoOp { current });
                    }
                },
                None => None,
            };
            // Without an active candidate, no rule's requirements
            // can be evaluated meaningfully — the only valid move
            // is to capture a candidate. The runtime caller
            // reads `next_actions` separately; reconcile itself
            // returns `NoOp` so the caller doesn't loop.
            if active_fp.is_none() {
                return Ok(ReconcileOutcome::NoOp { current });
            }
            for (idx, rule) in ironmaint_state::TRANSITION_RULES.iter().enumerate() {
                if rule.from != current {
                    continue;
                }
                // Build minimal ctx for evaluation.
                let gate_ids = self
                    .store
                    .list_gates_for_job(job_id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, format!("{e}")))?;
                let mut gate_defs = std::collections::BTreeMap::new();
                let mut gate_results = std::collections::BTreeMap::new();
                for gid in &gate_ids {
                    if let Ok(def) = self.store.get_gate_definition(*gid).await {
                        gate_defs.insert(*gid, def);
                    }
                    if let Some(fp) = active_fp.as_ref()
                        && let Ok(gres) = self.store.get_gate_result(*gid, fp).await
                    {
                        gate_results.insert(*gid, gres);
                    }
                }
                let ob_ids = self
                    .store
                    .list_obligations_for_job(job_id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, format!("{e}")))?;
                let mut obligations = ironmaint_policy::ObligationSet::new();
                for oid in &ob_ids {
                    if let Ok(ob) = self.store.get_obligation(*oid).await {
                        obligations.insert(ob);
                    }
                }
                let approvals = std::collections::BTreeMap::new();
                let approval_requirements = std::collections::BTreeMap::new();
                let ctx = ironmaint_state::TransitionContext {
                    current: &projection,
                    active_candidate_fingerprint: active_fp.clone(),
                    gates: &gate_results,
                    gate_definitions: &gate_defs,
                    obligations: &obligations,
                    approval_requirements: &approval_requirements,
                    approvals: &approvals,
                    // The same constant as the other production call
                    // site, deliberately: see `INFRASTRUCTURE_BLOCKED`.
                    infrastructure_blocked: INFRASTRUCTURE_BLOCKED,
                    resume_event: None,
                };
                let request = ironmaint_state::TransitionRequest {
                    job_id,
                    expected_version: projection.version,
                    target: rule.to,
                    now: self.clock.now_utc(),
                };
                let decision = engine.evaluate(&request, &ctx);
                match decision {
                    TransitionDecision::Allowed(_) => {
                        allowed_candidate = Some((rule.to, idx));
                        break;
                    }
                    TransitionDecision::Blocked(blockers) => {
                        let needs_actor = blockers
                            .iter()
                            .any(|b| matches!(b, TransitionBlocker::MissingApproval { .. }));
                        if needs_actor {
                            needs_actor_candidate = Some((rule.to, blockers));
                        } else if blocked_candidate.is_none() {
                            blocked_candidate = Some((rule.to, blockers));
                        }
                    }
                }
            }
            if let Some((target, idx)) = allowed_candidate {
                match self
                    .try_transition(job_id, target, projection.version)
                    .await
                {
                    Ok(t) => {
                        // §41: "Reconciliation may continue through
                        // multiple trivially satisfied stages until
                        // reaching a state requiring [agent
                        // action]." So keep walking, re-reading the
                        // projection each time — it moved. If the
                        // walk ends without a stop reason of its
                        // own (the bound below), the last
                        // transition is what gets reported.
                        last_advance = Some(ReconcileOutcome::Advanced {
                            from: t.from,
                            to: t.to,
                            rule_index: idx,
                        });
                        continue;
                    }
                    Err(e) if e.kind == RuntimeErrorKind::ConcurrentModification => {
                        return Ok(ReconcileOutcome::ConcurrentModification);
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
            if let Some((target, blockers)) = needs_actor_candidate {
                return Ok(ReconcileOutcome::NeedsActorDecision {
                    current,
                    target,
                    blockers: format_blocker_list(&blockers),
                });
            }
            if let Some((target, blockers)) = blocked_candidate {
                return Ok(ReconcileOutcome::Blocked {
                    current,
                    target,
                    blockers: format_blocker_list(&blockers),
                });
            }
            // No rule applied for `current` — the walk has stopped.
            return Ok(ReconcileOutcome::NoOp { current });
        }
        // Bound reached without a stop reason. Unreachable while the
        // rule table is acyclic; reported as the last real transition
        // rather than as a no-op, because the projection *did* move.
        Ok(last_advance.unwrap_or(ReconcileOutcome::NoOp {
            current: JobState::EventDetected,
        }))
    }

    /// `CaptureCandidate`: persist a freshly minted `SourceCandidate`
    /// for a job. Idempotent on `(job_id, fingerprint)` — a second
    /// call with the same fingerprint returns the existing
    /// `CandidateId` without re-inserting. The candidate is **not**
    /// activated here; that is `SetActiveCandidate`'s job.
    ///
    /// Implements PHASE-0B.md §6 (candidate persistence) + §45
    /// (capture boundary). Emits a `JobEvent::Domain` audit event
    /// describing the capture. Captures are valid in any state —
    /// the orchestrator decides when to capture, and the projection
    /// state advances through the state machine.
    async fn handle_capture_candidate(
        &self,
        job_id: JobId,
        candidate: SourceCandidate,
    ) -> Result<CommandResult, RuntimeError> {
        // D-21 closure (Phase 1 §4): `candidate.capture` must never
        // create a job implicitly. The job is the outer transaction
        // and the agent must call `job.create` first. We pre-check
        // the projection here so the agent gets the same typed
        // `InvalidInput` signal whether the backing store enforces
        // referential integrity (`SqliteStore`, which would
        // otherwise surface a raw `FOREIGN KEY constraint failed`)
        // or does not (`MockStore`, which would otherwise silently
        // accept the orphan). The downstream `activate_candidate`
        // also reads the projection — this check fails earlier and
        // with a typed error so an agent never sees a partial write.
        match self.store.get_projection(job_id).await {
            Ok(_) => {}
            Err(_) => {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::InvalidInput,
                    format!(
                        "cannot capture candidate for unknown job {job_id}: \
                         the agent must call `job.create` first"
                    ),
                ));
            }
        }

        // Reject candidates whose job_id does not match the
        // command's target — keeps the (job_id, fingerprint)
        // index invariant honest.
        if candidate.job_id() != job_id {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "candidate.job_id ({:?}) does not match command job_id ({:?})",
                    candidate.job_id(),
                    job_id
                ),
            ));
        }

        // Idempotent insert: if a candidate with this fingerprint
        // already exists, reuse its id rather than mint a new row.
        let candidate_id = if let Some(existing) = self
            .store
            .find_source_by_fingerprint(candidate.fingerprint())
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
        {
            existing
        } else {
            self.store
                .put_source_candidate(&candidate)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
        };

        // Audit event: surface the capture into the per-job event
        // log so §72 (persistence recovery) and §73 (projection
        // reconstruction) can reconstruct what was captured and
        // when.
        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let domain_event_id = DomainEventId::new();
        let now = self.clock.now_utc();
        let envelope = EventEnvelope::new(
            MaintenanceEventId::from_uuid(domain_event_id.as_uuid()),
            job_id,
            sequence,
            now,
            JobEvent::Domain(domain_event_id),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let side_effect = format!("source_candidate:{candidate_id} captured for job {job_id}");
        let mut side_effects = vec![side_effect];
        side_effects.extend(self.derive_candidate_plans(job_id, &candidate).await?);

        Ok(CommandResult {
            new_version: 0,
            new_sequence: sequence,
            side_effects,
        })
    }

    /// Derive the durable contracts a captured candidate needs, in
    /// the one place a candidate comes into existence.
    ///
    /// Capture is the *only* production path that reaches
    /// `SetActiveCandidate`, `MaterializeChecks`, and
    /// `put_obligation`. Before this existed all three were
    /// reachable from tests alone, so an orchestrator driving the
    /// tool surface could capture candidates forever and never
    /// leave `EventDetected` (D-03 in `doc/DEBT.md`).
    ///
    /// Three steps, each independently idempotent:
    ///
    /// 1. **Activate** the candidate, so `job.get` and
    ///    `reconcile` know what they are evaluating.
    /// 2. **Materialise checks** from the adapter's
    ///    `build_plan` / `qa_plan`, minting one `GateDefinition`
    ///    per planned check.
    /// 3. **Derive obligations** from the adapter's
    ///    `PolicyPlan`, one `Obligation` per template.
    ///
    /// Idempotence matters because `CaptureCandidate` is
    /// itself idempotent on fingerprint: re-capturing an existing
    /// candidate must not mint a second `GateDefinition` for the
    /// same stage (`GateDefinition::new` auto-mints its id, so
    /// there is no natural uniqueness key to fall back on) or a
    /// duplicate set of obligations.
    ///
    /// Returns human-readable notes for the command's
    /// `side_effects`. Deriving nothing is a *success* with a
    /// note, not an error: a family with no registered adapter is
    /// a legitimate state in 0B, and failing the capture would
    /// make the candidate unrepresentable.
    async fn derive_candidate_plans(
        &self,
        job_id: JobId,
        candidate: &SourceCandidate,
    ) -> Result<Vec<String>, RuntimeError> {
        let fingerprint = candidate.fingerprint().clone();

        // Activation first, and unconditionally. Whether a
        // candidate is *the* candidate is a fact about the job,
        // not a function of whether an adapter exists for its
        // distribution: `reconcile` and `job.get` both need it
        // either way. Deriving is the adapter-dependent half.
        let mut notes = self.activate_candidate(job_id, &fingerprint).await?;

        let family = &candidate.package().package.distribution.family;
        let Some(adapter) = self.adapters.get(family) else {
            notes.push(format!(
                "no adapter registered for family `{family}`; \
                 no checks or obligations derived for fingerprint {fingerprint}"
            ));
            return Ok(notes);
        };

        // Checks. `list_checks_for_job` is per job, so filter to
        // this candidate: a repaired job legitimately holds checks
        // for several fingerprints, and only the active one's gates
        // may be materialised.
        let check_ids = self
            .store
            .list_checks_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let mut already_planned = false;
        for check_id in check_ids {
            match self.store.get_check(check_id).await {
                Ok(check) if check.candidate == fingerprint => {
                    already_planned = true;
                    break;
                }
                Ok(_) => {}
                // A check row we cannot read is a store
                // inconsistency, not a reason to duplicate the
                // plan. Skip it and let the unique constraint
                // speak if it matters.
                Err(_) => {}
            }
        }

        if already_planned {
            notes.push(format!(
                "checks already materialised for fingerprint {fingerprint}; left unchanged"
            ));
        } else {
            let planned = self.plan_checks(adapter.as_ref(), candidate)?;
            // Only narrate a real materialisation.
            // `handle_materialize_checks` reports the count
            // itself, and an adapter that plans nothing is not
            // news.
            if !planned.is_empty() {
                notes.extend(
                    self.handle_materialize_checks(job_id, fingerprint.clone(), planned)
                        .await?
                        .side_effects,
                );
            }
        }

        // Obligations. Same shape: derive from the adapter's
        // PolicyPlan, once per candidate.
        let obligation_ids = self
            .store
            .list_obligations_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let mut obligations_exist = false;
        for obligation_id in obligation_ids {
            match self.store.get_obligation(obligation_id).await {
                Ok(obligation) if obligation.candidate == fingerprint => {
                    obligations_exist = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }

        if obligations_exist {
            notes.push(format!(
                "obligations already derived for fingerprint {fingerprint}; left unchanged"
            ));
        } else {
            let obligations = self.plan_obligations(adapter.as_ref(), candidate, &fingerprint)?;
            let count = obligations.len();
            for obligation in &obligations {
                self.store
                    .put_obligation(obligation, job_id)
                    .await
                    .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            }
            if count > 0 {
                notes.push(format!(
                    "derived {count} obligation(s) for fingerprint {fingerprint}"
                ));
            }
        }

        Ok(notes)
    }

    /// Attach `fingerprint` to the job as its active candidate,
    /// unless it already is.
    ///
    /// The ineligible-state case is reported, not raised. A
    /// candidate captured after the job has advanced past
    /// `CandidateAssembly` is real — that is the repair loop — but
    /// re-entry is a state-machine question (0A §21) that the
    /// runtime does not answer by fiat. Recording the fact in
    /// `side_effects` keeps it visible without this function
    /// claiming authority it does not have.
    async fn activate_candidate(
        &self,
        job_id: JobId,
        fingerprint: &CandidateFingerprint,
    ) -> Result<Vec<String>, RuntimeError> {
        let current = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // `active_candidate` is a `CandidateId`, but the command
        // carries a fingerprint. Resolve one to the other through
        // the store rather than comparing unlike types: a
        // re-capture is identified by its fingerprint, and the
        // fingerprint is the immutable identity of the source
        // revision.
        let already_active = match self.store.find_source_by_fingerprint(fingerprint).await {
            Ok(Some(candidate_id)) => current.active_candidate.as_ref() == Some(&candidate_id),
            Ok(None) => false,
            Err(e) => {
                return Err(RuntimeError::new(RuntimeErrorKind::Store, e.to_string()));
            }
        };
        if already_active {
            return Ok(vec![format!(
                "fingerprint {fingerprint} was already the active candidate"
            )]);
        }

        match self
            .handle_set_active_candidate(job_id, fingerprint.clone())
            .await
        {
            Ok(result) => {
                let mut notes = result.side_effects;
                notes.push(format!("activated fingerprint {fingerprint}"));
                Ok(notes)
            }
            Err(e) if e.kind == RuntimeErrorKind::InvalidInput => {
                // Report why. `handle_set_active_candidate` owns
                // the reason, including the remedy where one
                // exists ("resume the job first"), so an agent
                // that captures a patched candidate against a job
                // awaiting review can tell "my capture was inert"
                // from "I am expected to resume first". Appending
                // a second explanation here would say it twice
                // with different wording.
                Ok(vec![format!(
                    "captured fingerprint {fingerprint} but did not activate it: {}",
                    e.message
                )])
            }
            Err(e) => Err(e),
        }
    }

    /// Ask `adapter` for the build and QA checks this candidate
    /// requires, flattened into the tuples
    /// [`handle_materialize_checks`] consumes.
    ///
    /// Both phases are optional on the capability: an adapter may
    /// plan builds but not QA. The `AdapterDescriptor` advertises
    /// which, but `build()` returning `None` is the authoritative
    /// signal and is what the trait doc requires, so that is what
    /// is honoured here.
    fn plan_checks(
        &self,
        adapter: &dyn ironmaint_adapter_api::DistributionAdapter,
        candidate: &SourceCandidate,
    ) -> Result<Vec<(ToolCapabilityKey, ironmaint_evidence::EvidenceKind, bool)>, RuntimeError>
    {
        // PHASE-1.md §12: "The runtime aggregates inspection checks
        // during candidate capture exactly as it already aggregates
        // build/QA checks." The signature of `handle_materialize_checks`
        // is unchanged: every capability contributes tuples into the
        // same stream, and the materialise step doesn't care which
        // capability each tuple came from. An adapter that advertises
        // only `SourceInspection` (no build capability) still has its
        // inspection checks materialised; an adapter that advertises
        // only `BuildPlanning` still has its build checks materialised
        // — the early-return on missing `build()` is intentionally
        // gone, replaced by per-capability `if let Some(...)` blocks
        // that contribute zero tuples when the capability is absent.
        let mut out: Vec<(ToolCapabilityKey, ironmaint_evidence::EvidenceKind, bool)> = Vec::new();
        let ctx = ironmaint_adapter_api::contexts::CandidateContext {
            package: candidate.package(),
            candidate,
        };
        if let Some(build) = adapter.build() {
            let build_plan = build.build_plan(&ctx).map_err(adapter_error)?;
            let qa_plan = build.qa_plan(&ctx).map_err(adapter_error)?;
            out.extend(
                build_plan
                    .checks
                    .into_iter()
                    .chain(qa_plan.checks)
                    .map(|p| (p.key, p.evidence_kind, p.mandatory)),
            );
        }
        if let Some(inspection) = adapter.inspection() {
            let inspection_plan = inspection.inspection_plan(&ctx).map_err(adapter_error)?;
            out.extend(
                inspection_plan
                    .checks
                    .into_iter()
                    .map(|p| (p.key, p.evidence_kind, p.mandatory)),
            );
        }
        Ok(out)
    }

    /// Ask `adapter`'s policy capability for the obligations this
    /// candidate carries, bound to `fingerprint` and starting at
    /// `NotEvaluated`.
    ///
    /// The runtime holds no `PolicyBaseline` store, so
    /// `requested_baseline` is `None` and the adapter derives its
    /// own from the candidate's distribution. That matches what
    /// the conformance suite passes and is the honest answer for
    /// 0B: a baseline that no policy document produced should not
    /// be fabricated here.
    fn plan_obligations(
        &self,
        adapter: &dyn ironmaint_adapter_api::DistributionAdapter,
        candidate: &SourceCandidate,
        fingerprint: &CandidateFingerprint,
    ) -> Result<Vec<ironmaint_policy::Obligation>, RuntimeError> {
        let Some(policy) = adapter.policy() else {
            return Ok(Vec::new());
        };
        let ctx = ironmaint_adapter_api::contexts::PolicyContext {
            package: candidate.package(),
            candidate,
            requested_baseline: None,
        };
        let plan = policy.derive_obligation_plan(&ctx).map_err(adapter_error)?;
        plan.obligation_templates
            .into_iter()
            .map(|template| {
                ironmaint_policy::Obligation::new(
                    fingerprint.clone(),
                    template.reference,
                    template.strength,
                    template.applicability,
                    template.requirement,
                )
                .map_err(|reason| {
                    RuntimeError::new(
                        RuntimeErrorKind::InvalidInput,
                        format!(
                            "adapter {} produced an unusable obligation: {reason}",
                            adapter.descriptor().implementation_name
                        ),
                    )
                })
            })
            .collect()
    }

    /// `MaterializeChecks`: convert adapter `PlannedCheck`s into
    /// durable `GateDefinition` + `CheckDefinition` rows tied to
    /// the active candidate. Pure projection materialisation —
    /// does not transition state, only writes the durable
    /// contracts `RunCheck { check_id }` and `next_actions` will
    /// reference.
    ///
    /// Implements PHASE-0B.md §44 (Check Model) + §45 (Check
    /// Planning). Maps each `evidence_kind` to its
    /// `GateStage` via `gate_stage_for`; kinds that don't map
    /// to a known gate stage are silently filtered (e.g.,
    /// `EvidenceKind::Other(_)` is reserved for tool outputs
    /// that don't tie into a specific gate).
    async fn handle_materialize_checks(
        &self,
        job_id: JobId,
        candidate: CandidateFingerprint,
        planned: Vec<(ToolCapabilityKey, ironmaint_evidence::EvidenceKind, bool)>,
    ) -> Result<CommandResult, RuntimeError> {
        let materialised = crate::check::plan_to_materialised(candidate.clone(), &planned);
        let checks = crate::check::persist_materialised(
            self.store.as_ref(),
            job_id,
            candidate,
            &materialised,
        )
        .await
        .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let side_effect = format!("materialized {} check(s) for job {}", checks.len(), job_id);
        Ok(CommandResult {
            new_version: 0,
            new_sequence: 0,
            side_effects: vec![side_effect],
        })
    }

    /// `SetActiveCandidate`: attach a `CandidateFingerprint` to the
    /// job's projection. The fingerprint must already exist in the
    /// candidate store (typically populated by a prior capture via
    /// `CaptureCandidate`); we look it up, mark it as the job's
    /// active source candidate, and bump the projection's version.
    /// Does **not** transition state — the orchestrator advances
    /// state through the state machine via other commands.
    async fn handle_set_active_candidate(
        &self,
        job_id: JobId,
        fingerprint: CandidateFingerprint,
    ) -> Result<CommandResult, RuntimeError> {
        let current = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // A candidate may be (re-)activated at any point before
        // the job is *decided*. §61's repair loop is "inspect
        // the failure; capture a new candidate; rerun the
        // required checks", and §101 walks that loop three
        // times — the last from `FinalValidation`, after the
        // mandatory policy obligation has failed. An earlier
        // guard allowed only the four pre-build states, which
        // made the repair loop unreachable: the build and QA
        // failures it repairs happen in `BuildValidation` and
        // `PackageQaValidation`, and §41's walk moves the job
        // there on its own.
        //
        // What must not happen is swapping the source out from
        // under a reviewer, so the line is `ReadyForApproval` —
        // the state that exists so a human looks at *this*
        // candidate. Before that, re-activation is safe because
        // of §30's binding: the new candidate has no gate
        // results of its own, so nothing the old one proved
        // carries across to authorise it.
        if current.state.is_decided() {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "cannot set active candidate in state {:?}; the job is past \
                     ReadyForApproval, so the source under review is committed",
                    current.state
                ),
            ));
        }

        // Refused for a different reason: a job awaiting human
        // intervention is not one an agent may quietly re-point
        // at new source. `EnterHumanReview` already tells the
        // caller to resume first.
        if current.state.is_exceptional() {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "cannot set active candidate in state {:?}; resume the job first",
                    current.state
                ),
            ));
        }

        // Look up the candidate row by fingerprint. The candidate
        // must have been captured previously (PHASE-0B.md §6) —
        // SetActiveCandidate does not mint a new `SourceCandidate`,
        // it only activates an existing one.
        let candidate_id = self
            .store
            .find_source_by_fingerprint(&fingerprint)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?
            .ok_or_else(|| {
                RuntimeError::new(
                    RuntimeErrorKind::InvalidInput,
                    format!(
                        "no SourceCandidate with fingerprint {fingerprint}; \
                         capture it first"
                    ),
                )
            })?;

        // Mark the candidate active in the candidate store. This is
        // idempotent at the store level.
        self.store
            .set_active_source_candidate(job_id, candidate_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let now = self.clock.now_utc();

        let updated = JobProjection {
            job: current.job.clone(),
            state: current.state,
            active_candidate: Some(candidate_id),
            version: current.version + 1,
            updated_at: now,
        };

        self.store
            .put_projection(&updated, current.version)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // `CandidateActivated`, **not** a bare `Domain` id. The
        // three facts written into the row above — the candidate,
        // the version this CAS left behind, and the timestamp —
        // have to reach the log too, or the log is not the
        // authority §36 says it is. `Domain` is a bare audit
        // back-reference and `ProjectionApply::apply` treats it as
        // a no-op, so a replay through it dropped all three on every
        // single activation (D-16). A replay that dropped the active
        // candidate would not degrade the row; §30 makes every gate
        // verdict depend on that binding, so it would invalidate
        // the evidence for every check already run against the job.
        let activation = CandidateActivated {
            candidate_id,
            fingerprint: fingerprint.clone(),
            version_after: updated.version,
            updated_at: now,
        };
        let envelope = EventEnvelope::new(
            MaintenanceEventId::new(),
            job_id,
            sequence,
            now,
            JobEvent::CandidateActivated(activation),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        Ok(CommandResult {
            new_version: updated.version,
            new_sequence: sequence,
            side_effects: vec![],
        })
    }

    /// `RecordCheckEvidence`: persist an evidence record
    /// produced by an out-of-band agent (IronClaw, a human, a
    /// third-party CI). The job must have an active candidate.
    /// The projection's `version` is **not** bumped: evidence is
    /// a side artefact of the workflow, not a state transition.
    async fn handle_record_check_evidence(
        &self,
        job_id: JobId,
        _tool_key: String,
        evidence_kind: ironmaint_evidence::EvidenceKind,
        status: ironmaint_evidence::EvidenceStatus,
        producer: String,
    ) -> Result<CommandResult, RuntimeError> {
        let current = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let candidate_id = current.active_candidate.ok_or_else(|| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "no active candidate on job {job_id}; \
                     capture a candidate first"
                ),
            )
        })?;
        let source = self
            .store
            .get_source_candidate(candidate_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let fingerprint = source.fingerprint().clone();

        let now = self.clock.now_utc();
        let scope = EvidenceScope::Candidate(fingerprint.clone());
        let producer_obj = EvidenceProducer::new(producer);
        let evidence = Evidence::new(fingerprint, evidence_kind, status, producer_obj, scope, now);

        self.store
            .put_evidence(&evidence, job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let domain_event_id = DomainEventId::new();
        let envelope = EventEnvelope::new(
            MaintenanceEventId::from_uuid(domain_event_id.as_uuid()),
            job_id,
            sequence,
            now,
            JobEvent::Domain(domain_event_id),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        Ok(CommandResult {
            new_version: current.version,
            new_sequence: sequence,
            side_effects: vec![],
        })
    }

    /// `RecordObligationOutcome`: write the verdict of a policy
    /// evaluation onto one obligation. The `obligation_ref` is
    /// matched against the obligation's `requirement` text (the
    /// human-readable assertion), exactly and case-sensitively. The
    /// job must have an active candidate — the obligation is bound
    /// to a fingerprint, and an obligation belonging to a superseded
    /// candidate is not this job's current policy surface.
    async fn handle_record_obligation_outcome(
        &self,
        job_id: JobId,
        obligation_ref: String,
        outcome: ObligationOutcome,
    ) -> Result<CommandResult, RuntimeError> {
        let current = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let candidate_id = current.active_candidate.ok_or_else(|| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("no active candidate on job {job_id}; cannot satisfy obligation"),
            )
        })?;
        let source = self
            .store
            .get_source_candidate(candidate_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let fingerprint = source.fingerprint().clone();

        let obligation_ids = self
            .store
            .list_obligations_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // Find the obligation whose requirement text matches the
        // caller-supplied ref. Matches are exact (case-sensitive).
        let mut matched = None;
        for id in obligation_ids {
            let mut obligation = self
                .store
                .get_obligation(id)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            if obligation.requirement == obligation_ref && obligation.candidate == fingerprint {
                // An approved exception is backed by an explicit
                // approval record (0A §35). Re-evaluating the
                // assertion and recording a fresh verdict here would
                // silently revoke that approval without anyone
                // deciding to — a policy evaluation is not authority
                // to withdraw authority. Refuse instead.
                if obligation.status == ObligationStatus::ExceptionApproved {
                    return Err(RuntimeError::new(
                        RuntimeErrorKind::InvalidInput,
                        format!(
                            "obligation {} carries an approved exception; record an \
                             approval decision to change it, not an evaluation outcome",
                            obligation.id
                        ),
                    ));
                }
                obligation.status = outcome.as_status();
                matched = Some(obligation);
                break;
            }
        }

        let obligation = matched.ok_or_else(|| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "no obligation on job {job_id} with requirement {obligation_ref:?} for active candidate"
                ),
            )
        })?;

        // Written whole, so the obligation's existing evidence list
        // is carried across untouched: a verdict records what the
        // evaluation concluded, not which evidence it replaced.
        self.store
            .update_obligation(obligation.id, &obligation)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let now = self.clock.now_utc();
        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let domain_event_id = DomainEventId::new();
        let envelope = EventEnvelope::new(
            MaintenanceEventId::from_uuid(domain_event_id.as_uuid()),
            job_id,
            sequence,
            now,
            JobEvent::Domain(domain_event_id),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // The obligation is not moved by this. Policy completion is
        // gated by the transition engine, which reads the status on
        // the next reconcile or attempt; a job parks where it is and
        // reports the blocking obligation through `next_actions`.
        Ok(CommandResult {
            new_version: current.version,
            new_sequence: sequence,
            side_effects: vec![format!(
                "obligation:{} recorded {} on job:{job_id}",
                obligation.id,
                outcome.name(),
            )],
        })
    }

    /// Plan-aware `RunCheck { check_id }` (§15, §41, §44).
    ///
    /// Looks up the `CheckDefinition` by id, validates the
    /// candidate-attachment invariant (§30), looks up the tool
    /// registry entry, executes the tool, persists the evidence,
    /// and evaluates the corresponding gate (§29).
    ///
    /// Unknown `check_id`, candidate mismatch, and pre-capture
    /// states all reject with `RuntimeErrorKind::InvalidInput`.
    /// After execution the handler persists a `GateResult` for
    /// the check's gate.
    async fn handle_run_check(
        &self,
        job_id: JobId,
        check_id: ironmaint_core::CheckId,
        retry_class: ironmaint_executor::RetryClass,
    ) -> Result<CommandResult, RuntimeError> {
        let current = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let candidate_id = current.active_candidate.ok_or_else(|| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("no active candidate on job {job_id}; cannot run check"),
            )
        })?;
        let source = self
            .store
            .get_source_candidate(candidate_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let fingerprint = source.fingerprint().clone();

        // §30 evidence freshness: reject if the check is bound to
        // a candidate that is no longer active. This is the path
        // that triggers re-materialisation after a candidate
        // switch.
        let check = match self.store.get_check(check_id).await {
            Ok(c) => c,
            Err(e) if *e.kind() == ironmaint_store::StoreErrorKind::NotFound => {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::InvalidInput,
                    format!("unknown check {check_id}"),
                ));
            }
            Err(e) => {
                return Err(RuntimeError::new(RuntimeErrorKind::Store, e.to_string()));
            }
        };
        if check.job_id != job_id {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("check {check_id} does not belong to job {job_id}"),
            ));
        }
        if check.candidate != fingerprint {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "check {check_id} candidate {} != active fingerprint {}",
                    check.candidate, fingerprint
                ),
            ));
        }
        if !post_capture_state(current.state) {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "job {job_id} in state {:?} cannot run checks; need post-capture state",
                    current.state
                ),
            ));
        }

        let cap_key = check.capability.clone();
        let tool = self.registry.get(&cap_key).ok_or_else(|| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "no tool registered for capability {key}",
                    key = cap_key.as_str()
                ),
            )
        })?;

        // PHASE-1.md §7: a tool that declares `ToolInputMode::JsonStdin`
        // receives the runtime-built candidate context on its
        // stdin. The default mode (`None`) keeps the PHASE-0B §22
        // behaviour where the child gets `Stdio::null()` and the
        // payload is `Value::Null`. The fixture keys registered in
        // 0B do not opt in, so this branch is dormant until the
        // first Phase 1 inspection tool lands in 1C.x.
        let input = match tool.input_mode() {
            ironmaint_executor::ToolInputMode::None => serde_json::Value::Null,
            ironmaint_executor::ToolInputMode::JsonStdin => {
                build_candidate_input(&source, &check, self.workspace.as_deref()).await?
            }
        };

        let request = ExecutionRequest::new(job_id, cap_key.clone(), retry_class, input);

        let started_at = self.clock.now_utc();
        // A real executor reports a wall-clock timeout and a
        // spawn/wait failure as *errors*, not as records with a
        // non-zero exit. Propagating them with `?` aborted before
        // any evidence was written, which left the gate
        // `NotEvaluated` forever and made the Timeout /
        // InfrastructureFailed arms of the mapping below
        // unreachable — `ProcessExecutor` never returns a record
        // for either case. Synthesise the record the executor would
        // have produced and pin the outcome, so a timeout still
        // records `Fail` evidence and an infrastructure failure
        // still records `InfrastructureError` (PHASE-0B.md §32).
        let (record, forced_outcome) = match self.executor.execute(request).await {
            Ok(record) => (record, None),
            Err(e) => {
                let outcome = match e.kind {
                    ironmaint_executor::ExecutorErrorKind::ToolFailed { timed_out: true } => {
                        ironmaint_executor::Outcome::Timeout
                    }
                    ironmaint_executor::ExecutorErrorKind::InfrastructureFailed => {
                        ironmaint_executor::Outcome::InfrastructureFailed
                    }
                    // Every other kind is a genuine runtime
                    // failure with no domain verdict to record.
                    _ => {
                        return Err(RuntimeError::new(RuntimeErrorKind::Executor, e.to_string()));
                    }
                };
                (
                    ironmaint_executor::ExecutionRecord {
                        tool_key: cap_key.clone(),
                        retry_class,
                        started_at,
                        finished_at: self.clock.now_utc(),
                        // Not a real process exit; the pinned
                        // `forced_outcome` below is what drives
                        // classification, not this field.
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: e.to_string(),
                        retries_exhausted: false,
                        truncated: false,
                        // The executor was not reached, so no
                        // output was produced to spill.
                        artifacts: Vec::new(),
                        artifacts_dropped: Vec::new(),
                    },
                    Some(outcome),
                )
            }
        };

        // Translate executor-owned `Outcome` (PHASE-0B.md §32) into
        // the canonical evidence-status vocabulary. The mapping
        // collapses Timeout / Interrupted to Fail (they are tool-side
        // outcomes, not infrastructure problems) and lifts
        // InfrastructureFailed into the dedicated evidence status so
        // gate evaluation can react accordingly.
        //
        // PHASE-1.md §7: a tool that declares a `ResultNormalizer`
        // is authoritative — the normalizer's `evidence_status`
        // drives the Evidence row, not the exit-code fallback. A
        // `NormalizationError` is a tool-level outcome (the tool
        // emitted something the runtime could not classify), so it
        // becomes `Fail` with the producer tagged `#normalizer-error`
        // — distinct from the executor-error tag below, so an
        // operator can tell them apart. The normalizer is **not**
        // consulted when the executor never reached the tool
        // (`forced_outcome` is set): there is no `ExecutionRecord`
        // to normalise in that branch, and pretending the
        // normalizer ran would let a missing tool look like a
        // passing one.
        //
        // The `normalized` value is `Some(result)` when the
        // normalizer was consulted and accepted the record; the
        // runtime uses `r.evidence_status` directly and binds the
        // recorded stdout to the Evidence row as a `Report`
        // artifact (PHASE-1.md §31 — "store it as a report
        // artifact"). When the normalizer was not consulted (no
        // `ResultNormalizer` on the tool, or `forced_outcome`
        // set) the value is `None` and the existing exit-code
        // mapping runs.
        // PHASE-1.md §7: a tool that declares a `ResultNormalizer`
        // is authoritative — the normalizer's `evidence_status`
        // drives the Evidence row, not the exit-code fallback. A
        // `NormalizationError` is a tool-level outcome (the tool
        // emitted something the runtime could not classify), so it
        // becomes `Fail` with the producer tagged `#normalizer-error`
        // — distinct from the executor-error tag below, so an
        // operator can tell them apart. The normalizer is **not**
        // consulted when the executor never reached the tool
        // (`forced_outcome` is set): there is no `ExecutionRecord`
        // to normalise in that branch, and pretending the
        // normalizer ran would let a missing tool look like a
        // passing one.
        //
        // The `normalized` value is `Some(result)` when the
        // normalizer was consulted and accepted the record; the
        // runtime uses `r.evidence_status` directly and binds the
        // recorded stdout to the Evidence row as a `Report`
        // artifact (PHASE-1.md §31). When the normalizer was not
        // consulted (no `ResultNormalizer` on the tool, or
        // `forced_outcome` set) the value is `None` and the
        // existing exit-code mapping runs.
        let (outcome, normalized, normalizer_failure_note): (
            ironmaint_executor::Outcome,
            Option<ironmaint_executor::NormalizedResult>,
            Option<String>,
        ) = match forced_outcome {
            Some(o) => (o, None, None),
            None => match tool.normalizer() {
                Some(n) => match n.normalize(&record) {
                    Ok(r) => {
                        // Map the normalizer's `evidence_status` back
                        // to the executor-owned `Outcome` so the
                        // downstream `outcome_to_state` call (which
                        // drives the state-machine event) sees a
                        // consistent shape. A normalizer's
                        // `RequiresReview` / `NotEvaluated` falls
                        // through to `Fail` — the agent can read
                        // the Evidence row to see the nuance, and
                        // the state machine's `Fail` arm is the
                        // right hook for a human review.
                        let o = match r.evidence_status {
                            ironmaint_evidence::EvidenceStatus::Pass => {
                                ironmaint_executor::Outcome::Pass
                            }
                            _ => ironmaint_executor::Outcome::Fail,
                        };
                        (o, Some(r), None)
                    }
                    Err(e) => {
                        // A NormalizationError is a tool-level
                        // failure: the tool emitted something the
                        // runtime could not classify. Fail, not
                        // InfrastructureError, because the tool
                        // ran to completion and the runtime
                        // received its output.
                        (
                            ironmaint_executor::Outcome::Fail,
                            None,
                            Some(format!("{}#normalizer-error: {}", cap_key.as_str(), e)),
                        )
                    }
                },
                None => (ironmaint_executor::outcome_from_record(&record), None, None),
            },
        };

        // Map `Outcome` to `EvidenceStatus` after the precedence
        // resolution above, so there is exactly one mapping site
        // regardless of which path selected the outcome.
        let outcome_status = match outcome {
            ironmaint_executor::Outcome::Pass => ironmaint_evidence::EvidenceStatus::Pass,
            ironmaint_executor::Outcome::Fail => ironmaint_evidence::EvidenceStatus::Fail,
            ironmaint_executor::Outcome::Timeout => ironmaint_evidence::EvidenceStatus::Fail,
            ironmaint_executor::Outcome::Interrupted => ironmaint_evidence::EvidenceStatus::Fail,
            ironmaint_executor::Outcome::InfrastructureFailed => {
                ironmaint_evidence::EvidenceStatus::InfrastructureError
            }
        };

        // PHASE-1.md §31: bind the normalized report (the tool's
        // stdout, when the normalizer accepted it) to the
        // Evidence row as a `Report` artifact. The artifact store
        // is optional — when the runtime was constructed without
        // one (the default in every existing test, which has no
        // on-disk artifact root), the binding degrades to
        // "record only", not "panic": the Evidence row is still
        // recorded, the operator just has no on-disk file to
        // fetch via 1A.4's `evidence.artifact.read`. The daemon
        // always wires one.
        let report_artifact: Option<ironmaint_evidence::ArtifactRef> = match normalized {
            Some(_) => match self.artifacts.as_ref() {
                Some(store) => match store.put_bytes(record.stdout.as_bytes()).await {
                    Ok(rec) => {
                        let digest = ironmaint_core::Digest::new(
                            ironmaint_core::DigestAlgorithm::Sha256,
                            rec.digest.as_str(),
                        )
                        .map_err(|e| {
                            RuntimeError::new(
                                RuntimeErrorKind::Store,
                                format!("artifact digest rejected: {e}"),
                            )
                        })?;
                        Some(
                            ironmaint_evidence::ArtifactRef::new(
                                ironmaint_core::ArtifactId::new(),
                                ironmaint_evidence::ArtifactKind::Report,
                                digest,
                            )
                            .with_size_bytes(rec.size)
                            .with_media_type("application/json"),
                        )
                    }
                    Err(e) => {
                        // An artifact-store write failure is an
                        // infrastructure problem, not a tool
                        // failure: surface it as such rather
                        // than pretending the report was bound.
                        return Err(RuntimeError::new(
                            RuntimeErrorKind::Store,
                            format!("artifact write failed: {e}"),
                        ));
                    }
                },
                None => None,
            },
            None => None,
        };

        let now = self.clock.now_utc();
        let scope = EvidenceScope::Candidate(fingerprint.clone());
        // Tag the producer with one of three suffixes so the
        // source of a `Fail` is distinguishable downstream.
        // `#executor-error` — the executor never reached the
        // tool (forced_outcome). `#normalizer-error` — the tool
        // ran and produced output the normalizer could not
        // classify. The bare capability key — the tool ran and
        // the result was either classified by the exit-code
        // fallback or by a normalizer that accepted the record.
        let producer_name = match (forced_outcome, normalizer_failure_note) {
            (Some(_), _) => format!("{}#executor-error", cap_key.as_str()),
            (None, Some(note)) => note,
            (None, None) => cap_key.as_str().to_string(),
        };
        let producer = EvidenceProducer::new(producer_name);
        // PHASE-0B.md §15: evidence rows must explicitly record
        // `truncated = true` when the executor bounded stdout/stderr.
        // PHASE-1.md §7: a successful normalizer may report the
        // output as truncated even when the executor didn't
        // (e.g. a structured report that itself marks a partial
        // capture); honor the normalizer's flag in that case.
        let truncated = match normalized {
            Some(r) => r.output_truncated,
            None => record.truncated,
        };
        let mut evidence = Evidence::new(
            fingerprint.clone(),
            check.evidence_kind.clone(),
            outcome_status,
            producer,
            scope,
            now,
        )
        .with_truncated(truncated);
        if let Some(artifact) = report_artifact {
            evidence = evidence.with_artifact(artifact);
        }

        self.store
            .put_evidence(&evidence, job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // §29 gate evaluation: walk all checks attached to the
        // gate this `check` belongs to, aggregate, persist
        // GateResult.
        let all_checks = self
            .store
            .list_checks_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let mut gate_checks: Vec<CheckDefinition> = Vec::with_capacity(all_checks.len());
        for cid in all_checks {
            let c = self
                .store
                .get_check(cid)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            if c.gate_id == check.gate_id {
                gate_checks.push(c);
            }
        }
        let gate_id = check.gate_id;
        let result = crate::check::evaluate_gate(
            &*self.store,
            gate_id,
            fingerprint.clone(),
            &gate_checks,
            now,
        )
        .await
        .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        self.store
            .put_gate_result(gate_id, &fingerprint, &result)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        // §48: "A deterministic fixture policy evaluator produces
        // the evidence." A policy-evaluation check is that
        // evaluator, so its evidence is what an obligation's
        // verdict is derived from — never a field a caller writes.
        // There is deliberately no MCP tool for this: §53 forbids
        // `ironmaint_obligation_set_pass` and §102 item 25 says the
        // same. Without this step a mandatory obligation could
        // only ever be `NotEvaluated` over the tool surface, and a
        // job could never reach `ReadyForApproval` (§101 step 28).
        let mut obligation_notes = self
            .derive_obligation_verdicts(job_id, &source, &check.evidence_kind, &evidence)
            .await?;

        let sequence = self
            .store
            .next_sequence(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let outcome_state = outcome_to_state(outcome);
        let tool_finished = ToolRunFinished::new(evidence.id, record.truncated, outcome_state, now);
        let envelope = EventEnvelope::new(
            MaintenanceEventId::new(),
            job_id,
            sequence,
            now,
            JobEvent::ToolRunFinished(tool_finished),
        );
        self.store
            .append_event(&envelope)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;

        let mut side_effects = vec![
            format!(
                "ran tool {cap} on job:{job}",
                cap = cap_key.as_str(),
                job = job_id
            ),
            format!("gate {gate_id} -> {status}", status = result.status),
        ];
        side_effects.append(&mut obligation_notes);
        Ok(CommandResult {
            new_version: current.version,
            new_sequence: sequence,
            side_effects,
        })
    }

    /// Record an obligation verdict for every obligation of the
    /// active candidate, when the check that just ran was the
    /// adapter's policy evaluator.
    ///
    /// Returns human-readable notes for the caller's
    /// `side_effects`, and nothing at all when the check was not a
    /// policy evaluation — the overwhelmingly common case.
    ///
    /// Why "every obligation of the candidate" rather than one
    /// named by the check: §48's evaluator judges a *policy*, and a
    /// policy plan can carry several obligations (the Debian stub
    /// derives two). A `PlannedCheck` names a tool, not a
    /// requirement, so there is nothing narrower to key on. What the
    /// adapter refuses — because the runtime only ever offers it
    /// templates this same adapter derived — is a requirement the
    /// adapter does not recognise.
    ///
    /// An obligation the evaluator declines to rule on
    /// (`verdict_from_evidence_status` returns an error for
    /// `Inconclusive` and `InfrastructureError`) is left
    /// `NotEvaluated` and reported as a note. That is the honest
    /// outcome: the gate stays shut, and the note says why.
    async fn derive_obligation_verdicts(
        &self,
        job_id: JobId,
        source: &ironmaint_core::SourceCandidate,
        evidence_kind: &ironmaint_evidence::EvidenceKind,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<Vec<String>, RuntimeError> {
        if *evidence_kind != ironmaint_evidence::EvidenceKind::PolicyEvaluation {
            return Ok(Vec::new());
        }
        let Some(adapter) = self
            .adapters
            .get(&source.package().package.distribution.family)
        else {
            return Ok(vec![format!(
                "no adapter registered for family `{}`; obligation verdicts not derived",
                source.package().package.distribution.family
            )]);
        };
        let Some(policy) = adapter.policy() else {
            return Ok(vec![format!(
                "adapter `{}` has no policy capability; obligation verdicts not derived",
                adapter.descriptor().implementation_name
            )]);
        };

        let context = ironmaint_adapter_api::PolicyContext {
            package: source.package(),
            candidate: source,
            requested_baseline: None,
        };
        let ids = self
            .store
            .list_obligations_for_job(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
        let mut notes = Vec::new();
        for id in ids {
            let obligation = self
                .store
                .get_obligation(id)
                .await
                .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
            if obligation.candidate != *source.fingerprint() {
                continue;
            }
            let template = ironmaint_adapter_api::ObligationTemplate::new(
                obligation.reference.clone(),
                obligation.strength,
                obligation.applicability,
                obligation.requirement.clone(),
            );
            match policy.evaluate_obligation(&context, &template, evidence) {
                Ok(outcome) => {
                    let mut updated = obligation.clone();
                    updated.status = outcome.as_status();
                    if !updated.evidence.contains(&evidence.id) {
                        updated.evidence.push(evidence.id);
                    }
                    self.store
                        .put_obligation(&updated, job_id)
                        .await
                        .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))?;
                    notes.push(format!(
                        "obligation {id} -> {:?} from {}",
                        outcome.as_status(),
                        evidence.producer.name
                    ));
                }
                Err(e) => {
                    // A declined verdict is not a failure of the
                    // check: the obligation stays `NotEvaluated`,
                    // the gate stays shut, and the reason is on the
                    // record.
                    notes.push(format!("obligation {id} left NotEvaluated: {}", e.message));
                }
            }
        }
        Ok(notes)
    }
}

/// Map an [`ironmaint_adapter_api::AdapterError`] onto the
/// runtime's error vocabulary.
///
/// The adapter's own kind is always preserved in the message.
/// Two kinds get a dedicated runtime kind because callers already
/// branch on them: `Unsupported` means the capability does not
/// exist (and must not be confused with "failed"), and the
/// malformed-package kinds mean the *input* was bad. Everything
/// else — a policy backend that could not be reached, an external
/// system that failed — is `Other` carrying the adapter's kind,
/// because none of them is a store, workspace, or executor fault
/// and none of them is the caller's fault either.
fn adapter_error(e: ironmaint_adapter_api::AdapterError) -> RuntimeError {
    use ironmaint_adapter_api::AdapterErrorKind as K;
    let kind = match e.kind {
        K::Unsupported => RuntimeErrorKind::Unsupported,
        K::InvalidPackage
        | K::InvalidVersion
        | K::InvalidConfiguration
        | K::MissingRequiredMetadata => RuntimeErrorKind::InvalidInput,
        K::PolicyUnavailable
        | K::HumanReviewRequired
        | K::ExternalTransient
        | K::ExternalPermanent
        | K::InternalAdapterFailure => RuntimeErrorKind::Other(e.kind.name().to_string()),
        // `AdapterErrorKind` is `#[non_exhaustive]`. A kind added
        // after this match is reported as `Other` carrying its own
        // name rather than folded into a neighbouring arm —
        // mislabelling a new failure mode is worse than an
        // imprecise one.
        _ => RuntimeErrorKind::Other(e.kind.name().to_string()),
    };
    RuntimeError::new(
        kind,
        format!("adapter error [{}]: {}", e.kind.name(), e.message),
    )
}

/// True if the job state can record evidence from a runtime
/// check.
///
/// Every state that is neither terminal nor exceptional qualifies.
/// Terminal states have nothing left to verify, and the two
/// exceptional states are waiting on a human or on infrastructure
/// — running a check would not resolve either.
///
/// **This used to exclude `EventDetected`..`CandidateAssembly`**, on
/// the grounds that "pre-capture states have no active candidate
/// fingerprint and cannot materialise checks". As of 0B.10 C1 that
/// reasoning is obsolete: `CaptureCandidate` now activates the
/// candidate and materialises its gates in the same call, so those
/// states routinely hold an active candidate with checks already
/// bound to it. Requiring the job to advance first would invert the
/// dependency — the job cannot advance without gate results, and
/// gate results cannot be produced without running checks.
///
/// The conditions that actually matter are enforced separately and
/// still are: `handle_run_check` rejects a job with no active
/// candidate, and rejects a check whose fingerprint is not the
/// active one (§30 evidence freshness).
fn post_capture_state(state: ironmaint_core::JobState) -> bool {
    use ironmaint_core::JobState as S;
    !matches!(
        state,
        S::Published | S::Cancelled | S::HumanReviewRequired | S::InfrastructureBlocked
    )
}

/// Translate the executor-owned `Outcome` enum (PHASE-0B.md §32) to
/// the canonical `ironmaint_state::ToolOutcome` variant used on the
/// event wire (§15, §92). State owns the wire vocabulary; the
/// executor's enum is internal and mirrors it 1:1.
fn outcome_to_state(outcome: ironmaint_executor::Outcome) -> ToolOutcome {
    match outcome {
        ironmaint_executor::Outcome::Pass => ToolOutcome::Pass,
        ironmaint_executor::Outcome::Fail => ToolOutcome::Fail,
        ironmaint_executor::Outcome::Timeout => ToolOutcome::Timeout,
        ironmaint_executor::Outcome::Interrupted => ToolOutcome::Interrupted,
        ironmaint_executor::Outcome::InfrastructureFailed => ToolOutcome::InfrastructureFailed,
    }
}

/// Build the JSON payload the runtime hands to a tool whose
/// `ToolInputMode` is `JsonStdin` (PHASE-1.md §7).
///
/// The shape is what the Phase 1 inspection tools (1C.x) need to
/// understand which candidate they are about to inspect: the
/// `job_id`, the `SourceCandidate`'s package family / name /
/// repository URL / commit OID / tree OID, the candidate's
/// BLAKE3 fingerprint, the check's own `id` (so a tool that
/// produces multiple artefacts can identify which gate's
/// evidence row the runtime will persist), and — from 1C.1
/// onward, in `ironmaint.candidate_input.v2` — the on-disk
/// path of the candidate's working tree.
///
/// `workspace`, when `Some`, lets the runtime compute the
/// per-job workspace directory the tool needs in order to read
/// the candidate's source files. It is `None` for tools whose
/// `ToolInputMode` is `None` (the tool never receives the
/// payload) and may also be `None` for tests that build the
/// service without a manager; in that case the
/// `workspace_path` field is omitted from the payload, and
/// any tool that depends on it reports
/// `verdict: "infrastructure_error"` (the §17 rule for
/// "workspace path missing"). The production daemon always
/// wires it.
///
/// A failure to serialise is reported as `RuntimeError::Store` —
/// serialising a fixed-shape value from already-validated
/// internals is not a tool-level outcome, and the runtime
/// should not pretend it is.
async fn build_candidate_input<S: IronMaintStore + ?Sized>(
    source: &SourceCandidate,
    check: &CheckDefinition,
    workspace: Option<&ironmaint_workspace::WorkspaceManager<Arc<S>>>,
) -> Result<serde_json::Value, RuntimeError> {
    use ironmaint_core::GitObjectId;

    let commit: &GitObjectId = source.commit();
    let tree: &GitObjectId = source.tree();
    let package = source.package();
    let repo = source.repository();

    // The v2 payload. The only addition over v1 is
    // `workspace_path`; every other field is identical so an
    // old tool that does not look at `workspace_path` keeps
    // working.
    let mut payload = serde_json::json!({
        "schema": "ironmaint.candidate_input.v2",
        "job_id": source.job_id().to_string(),
        "check_id": check.id.to_string(),
        "package": {
            "name": package.package.source_name.as_str(),
            "version": package.version.as_str(),
        },
        "repository": {
            "url": repo.url().as_str(),
        },
        "commit": commit.as_str(),
        "tree": tree.as_str(),
        "fingerprint": source.fingerprint().as_str(),
    });

    // Resolve the per-job working tree. `id_for_job` is the
    // store-backed lookup; the per-job handle is the directory
    // the runtime created (via `ensure_workspace`) when
    // `candidate.capture` activated the candidate. A lookup
    // failure is reported as a missing field — the tool then
    // reports `verdict: "infrastructure_error"`, which is the
    // §17 distinction from `verdict: "fail"` (a present-but-
    // wrong workspace).
    if let (Some(ws), Some(obj)) = (workspace, payload.as_object_mut()) {
        match ws.id_for_job(source.job_id()).await {
            Ok(id) => {
                let path = ws.root().join(format!("{id}"));
                obj.insert(
                    "workspace_path".to_string(),
                    serde_json::Value::String(path.display().to_string()),
                );
            }
            // No workspace for this job — leave the field out.
            // The tool's contract (PHASE-1.md §17) maps
            // "workspace_path missing" to
            // `verdict: "infrastructure_error"`, which is the
            // right answer for "the runtime did not materialise
            // a working tree, so the check cannot run."
            Err(_) => {}
        }
    }

    serde_json::to_value(payload)
        .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, e.to_string()))
}

/// Pure projection helper: maps the current `JobState` into the
/// actions the orchestrator can take right now. Real
/// next-action computation walks the evidence ledger; this
/// minimal version covers the §42 contract that the type
/// exists and serializes correctly.
///
/// In the rewrite for 0B.5 C5 (§42), the projection is now
/// walked against the evidence ledger, the per-candidate gate
/// definitions, and the per-job obligation set. The per-state
/// bucket table is preserved as the *fallback* when no
/// evidence-derived action can be produced — for example,
/// before any candidate is attached the only sensible move
/// is `CaptureCandidate`.
fn project_next_actions(job_id: JobId, state: ironmaint_core::JobState) -> JobNextActions {
    use ironmaint_core::JobState as S;
    match state {
        S::EventDetected | S::Intake | S::SourceReview | S::CandidateAssembly => JobNextActions {
            job_id,
            allowed: vec![AllowedAction::CaptureCandidate],
            requires_human: None,
            blockers: vec![],
        },
        S::SourceRevision
        | S::SourceIntegrity
        | S::BuildValidation
        | S::PackageQaValidation
        | S::FunctionalValidation
        | S::UpgradeValidation => JobNextActions {
            job_id,
            // `RunCheck` entries are attached by `handle_query`,
            // not here: which checks exist is a fact about the
            // store, not about the state alone.
            allowed: vec![],
            requires_human: None,
            // Which tool is pending is likewise a fact about the
            // store, so this pure projection cannot name one.
            blockers: vec![ActionBlocker::GatePending { tool_key: None }],
        },
        // Policy completion is a person's judgement.
        // `RecordObligationOutcome` has no MCP tool in 0B, and
        // §102 item 25 forbids adding one — "MCP cannot directly set
        // state, gates, obligations, approvals, or evidence". Which
        // obligations are actually outstanding is a fact about the
        // store, so `attach_obligation_state` fills that in.
        S::ReleaseReview | S::FinalValidation => JobNextActions {
            job_id,
            allowed: vec![],
            requires_human: Some(HumanAction::SatisfyObligation { reference: None }),
            blockers: vec![],
        },
        S::ReadyForApproval => JobNextActions {
            job_id,
            allowed: vec![],
            requires_human: Some(HumanAction::ApproveRelease),
            blockers: vec![],
        },
        S::Approved | S::PublicationPending => JobNextActions {
            job_id,
            allowed: vec![],
            requires_human: Some(HumanAction::AuthorizePublication),
            blockers: vec![],
        },
        S::Published | S::Cancelled => JobNextActions::empty(job_id),
        // Neither exceptional state waits on a person by definition
        // of the name — `HumanReviewRequired` does, but 0A §21
        // gives it no entry point, so the runtime cannot yet produce
        // this projection and would be inventing a state. `allowed`
        // is filled in from the log by `attach_resume_action` when a
        // resume record exists; the human is notified out of band.
        S::HumanReviewRequired => JobNextActions {
            job_id,
            allowed: vec![],
            requires_human: Some(HumanAction::ReviewEscalation),
            blockers: vec![],
        },
        S::InfrastructureBlocked => JobNextActions {
            job_id,
            allowed: vec![],
            requires_human: None,
            blockers: vec![ActionBlocker::StateMachineBlocked(
                "infrastructure_blocked".to_string(),
            )],
        },
    }
}

/// Format a list of [`TransitionBlocker`] into a `Vec<String>`,
/// one entry per blocker. Used by `reconcile` so the outcome
/// is serializable.
pub(crate) fn format_blocker_list(blockers: &[ironmaint_state::TransitionBlocker]) -> Vec<String> {
    blockers.iter().map(|b| format!("{b:?}")).collect()
}

/// Format a list of [`TransitionBlocker`] into a single-line
/// error message suitable for a runtime error display.
fn format_blockers(blockers: &[ironmaint_state::TransitionBlocker]) -> String {
    blockers
        .iter()
        .map(|b| match b {
            ironmaint_state::TransitionBlocker::InvalidStatePath => {
                "invalid state path".to_string()
            }
            ironmaint_state::TransitionBlocker::StaleProjection { expected, found } => {
                format!("stale projection (expected {expected}, found {found})")
            }
            ironmaint_state::TransitionBlocker::MissingGate(id) => {
                format!("missing gate {id}")
            }
            ironmaint_state::TransitionBlocker::FailedGate(id) => {
                format!("failed gate {id}")
            }
            ironmaint_state::TransitionBlocker::IncompleteGate(id) => {
                format!("incomplete gate {id}")
            }
            ironmaint_state::TransitionBlocker::MissingObligation(id) => {
                format!("missing obligation {id}")
            }
            ironmaint_state::TransitionBlocker::FailedObligation(id) => {
                format!("failed obligation {id}")
            }
            ironmaint_state::TransitionBlocker::ReviewRequired(id) => {
                format!("review-required obligation {id}")
            }
            ironmaint_state::TransitionBlocker::UnbackedException(id) => {
                format!("unbacked exception {id}")
            }
            ironmaint_state::TransitionBlocker::MissingApproval(id) => {
                format!("missing approval {id}")
            }
            ironmaint_state::TransitionBlocker::StaleEvidence(id) => {
                format!("stale evidence on approval {id}")
            }
            ironmaint_state::TransitionBlocker::CandidateMismatch { expected, found } => {
                format!("candidate mismatch (expected {expected}, found {found})")
            }
            ironmaint_state::TransitionBlocker::InfrastructureBlocked => {
                "infrastructure blocked".to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build the refusal for [`RuntimeCommand::RequestApproval`].
///
/// A free function rather than a method on purpose: the handler
/// has no `self`, so "this touches nothing" is a property the type
/// system can see instead of a claim in a comment. Nothing is read
/// and nothing is written — no projection bump, no event, no
/// approval record — so a refused request leaves the job exactly
/// as it found it and retrying is free.
fn refuse_approval(job_id: JobId, category: &ironmaint_policy::ApprovalCategory) -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::Unsupported,
        format!(
            "RequestApproval is refused for job {job_id} (category `{category}`): an \
             approval is a human principal's decision and the orchestrator that calls \
             this command must not be able to grant it. No approval store or out-of-band \
             delivery channel exists in 0B, so the request is not recorded. Leave the job \
             at ReadyForApproval; `job.reconcile` reports NeedsActorDecision until a human \
             acts outside this process."
        ),
    )
}

fn _assert_send_sync<T: Send + Sync>() {}
fn _assert_service_send_sync<
    S: IronMaintStore + Send + Sync + ?Sized,
    E: Executor + Send + Sync + ?Sized,
>() {
    _assert_send_sync::<JobId>();
    _assert_send_sync::<RuntimeService<S, E>>();
    fn _assert_arc_dyn_clock_send_sync() {
        let _: Arc<dyn Clock> = Arc::new(crate::clock::SystemClock);
    }
}
