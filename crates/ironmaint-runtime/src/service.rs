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
use ironmaint_policy::PrivilegedOperation;
use ironmaint_state::{
    JobEvent, ToolOutcome, ToolRunFinished, TransitionBlocker, TransitionDecision,
};
use ironmaint_store::CheckDefinition;
use ironmaint_store::IronMaintStore;
use ironmaint_store::envelope::EventEnvelope;

use crate::adapters::AdapterRegistry;
use crate::clock::Clock;
use crate::command::RuntimeCommand;
use crate::error::{RuntimeError, RuntimeErrorKind};
use crate::next_actions::{ActionBlocker, AllowedAction, JobNextActions};
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
}

pub struct RuntimeService<S: IronMaintStore + ?Sized, E: Executor + ?Sized> {
    store: Arc<S>,
    clock: Arc<dyn Clock>,
    executor: Arc<E>,
    registry: Arc<ToolRegistry>,
    adapters: AdapterRegistry,
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
        }
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
            RuntimeCommand::MarkObligationSatisfied {
                job_id,
                obligation_ref,
            } => {
                self.handle_mark_obligation_satisfied(job_id, obligation_ref)
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
        }
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

        // Seed the event log with a Domain event pointing at the
        // orchestrator that initiated the job. This is the very
        // first sequence number for this job.
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
            infrastructure_blocked: false,
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
    /// - bounded loop overflow — `NoOp` (defensive; never expected)
    pub async fn reconcile(&self, job_id: JobId) -> Result<ReconcileOutcome, RuntimeError> {
        #[allow(clippy::never_loop)]
        for _ in 0..15u32 {
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
                    infrastructure_blocked: false,
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
                        return Ok(ReconcileOutcome::Advanced {
                            from: t.from,
                            to: t.to,
                            rule_index: idx,
                        });
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
            // No rule applied for `current` — loop terminates.
            return Ok(ReconcileOutcome::NoOp { current });
        }
        // Defensive: shouldn't hit 15 iterations in practice.
        let projection = self
            .store
            .get_projection(job_id)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Store, format!("{e}")))?;
        Ok(ReconcileOutcome::NoOp {
            current: projection.state,
        })
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
                // Report why, and — when the reason is an
                // exceptional state — what would unblock it. The
                // underlying message names the state but not the
                // remedy, and an agent that captures a patched
                // candidate against a job awaiting review would
                // otherwise have no way to tell "my capture was
                // inert" from "I am expected to resume first".
                let remedy = if current.state.is_exceptional() {
                    format!(
                        " — job is in {}; resume it before a candidate can be activated",
                        current.state
                    )
                } else {
                    String::new()
                };
                Ok(vec![format!(
                    "captured fingerprint {fingerprint} but did not activate it: {}{remedy}",
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
        let Some(build) = adapter.build() else {
            return Ok(Vec::new());
        };
        let ctx = ironmaint_adapter_api::contexts::CandidateContext {
            package: candidate.package(),
            candidate,
        };
        let build_plan = build.build_plan(&ctx).map_err(adapter_error)?;
        let qa_plan = build.qa_plan(&ctx).map_err(adapter_error)?;
        Ok(build_plan
            .checks
            .into_iter()
            .chain(qa_plan.checks)
            .map(|p| (p.key, p.evidence_kind, p.mandatory))
            .collect())
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

        // Candidate capture is only valid in the four pre-build
        // states. Once we reach `SourceRevision` the candidate is
        // already committed.
        let allowed_states = [
            JobState::EventDetected,
            JobState::Intake,
            JobState::SourceReview,
            JobState::CandidateAssembly,
        ];
        if !allowed_states.contains(&current.state) {
            return Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "cannot set active candidate in state {:?}; expected one of {:?}",
                    current.state, allowed_states
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

    /// `MarkObligationSatisfied`: flip an obligation's status to
    /// `Pass`. The `obligation_ref` is matched against the
    /// obligation's `requirement` text (the human-readable
    /// assertion). The job must have an active candidate (the
    /// obligation is bound to a fingerprint).
    async fn handle_mark_obligation_satisfied(
        &self,
        job_id: JobId,
        obligation_ref: String,
    ) -> Result<CommandResult, RuntimeError> {
        use ironmaint_policy::ObligationStatus;

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
                obligation.status = ObligationStatus::Pass;
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

        // Preserve the existing evidence list.
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

        let _ = obligation.evidence; // silence unused warning if obligation.evidence is unused below
        Ok(CommandResult {
            new_version: current.version,
            new_sequence: sequence,
            side_effects: vec![format!(
                "obligation:{} marked satisfied on job:{job_id}",
                obligation.id
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
        let _tool = self.registry.get(&cap_key).ok_or_else(|| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!(
                    "no tool registered for capability {key}",
                    key = cap_key.as_str()
                ),
            )
        })?;

        let request = ExecutionRequest::new(cap_key.clone(), retry_class, serde_json::Value::Null);

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
        let outcome =
            forced_outcome.unwrap_or_else(|| ironmaint_executor::outcome_from_record(&record));
        let outcome_status = match outcome {
            ironmaint_executor::Outcome::Pass => ironmaint_evidence::EvidenceStatus::Pass,
            ironmaint_executor::Outcome::Fail => ironmaint_evidence::EvidenceStatus::Fail,
            ironmaint_executor::Outcome::Timeout => ironmaint_evidence::EvidenceStatus::Fail,
            ironmaint_executor::Outcome::Interrupted => ironmaint_evidence::EvidenceStatus::Fail,
            ironmaint_executor::Outcome::InfrastructureFailed => {
                ironmaint_evidence::EvidenceStatus::InfrastructureError
            }
        };

        let now = self.clock.now_utc();
        let scope = EvidenceScope::Candidate(fingerprint.clone());
        // When the record was synthesised rather than produced by a
        // real process, tag the producer so a timeout is
        // distinguishable from the tool legitimately exiting
        // non-zero. Both map to `Fail`, so this is the only signal
        // that separates them.
        let producer_name = match forced_outcome {
            Some(_) => format!("{}#executor-error", cap_key.as_str()),
            None => cap_key.as_str().to_string(),
        };
        let producer = EvidenceProducer::new(producer_name);
        // PHASE-0B.md §15: evidence rows must explicitly record
        // `truncated = true` when the executor bounded stdout/stderr.
        let evidence = Evidence::new(
            fingerprint.clone(),
            check.evidence_kind.clone(),
            outcome_status,
            producer,
            scope,
            now,
        )
        .with_truncated(record.truncated);

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

        Ok(CommandResult {
            new_version: current.version,
            new_sequence: sequence,
            side_effects: vec![
                format!(
                    "ran tool {cap} on job:{job}",
                    cap = cap_key.as_str(),
                    job = job_id
                ),
                format!("gate {gate_id} -> {status}", status = result.status),
            ],
        })
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
            blockers: vec![],
        },
        S::SourceRevision
        | S::SourceIntegrity
        | S::BuildValidation
        | S::PackageQaValidation
        | S::FunctionalValidation
        | S::UpgradeValidation => JobNextActions {
            job_id,
            allowed: vec![],
            // Which tool is pending is a fact about the store, not
            // about the state alone, so this pure projection cannot
            // name one. `handle_query` fills it in from the job's
            // materialised checks.
            blockers: vec![ActionBlocker::GatePending { tool_key: None }],
        },
        S::ReleaseReview | S::FinalValidation => JobNextActions {
            job_id,
            allowed: vec![AllowedAction::MarkObligationSatisfied],
            blockers: vec![],
        },
        S::ReadyForApproval => JobNextActions {
            job_id,
            allowed: vec![AllowedAction::RequestApproval],
            blockers: vec![],
        },
        S::Approved | S::PublicationPending => JobNextActions {
            job_id,
            allowed: vec![AllowedAction::AuthorizeOperation],
            blockers: vec![],
        },
        S::Published | S::Cancelled => JobNextActions::empty(job_id),
        S::HumanReviewRequired => JobNextActions {
            job_id,
            allowed: vec![],
            blockers: vec![ActionBlocker::ObligationPending {
                reference: "human".to_string(),
            }],
        },
        S::InfrastructureBlocked => JobNextActions {
            job_id,
            allowed: vec![],
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
