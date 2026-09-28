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

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, DomainEventId, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, SourceCandidate,
};
use ironmaint_evidence::{Evidence, EvidenceProducer, EvidenceScope};
use ironmaint_executor::{ExecutionRequest, Executor, ToolRegistry};
use ironmaint_state::{JobEvent, ToolOutcome, ToolRunFinished};
use ironmaint_store::CheckDefinition;
use ironmaint_store::IronMaintStore;
use ironmaint_store::envelope::EventEnvelope;

use crate::clock::Clock;
use crate::command::RuntimeCommand;
use crate::error::{RuntimeError, RuntimeErrorKind};
use crate::next_actions::{ActionBlocker, AllowedAction, JobNextActions};
use crate::query::RuntimeQuery;

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
}

pub struct RuntimeService<S: IronMaintStore + ?Sized, E: Executor + ?Sized> {
    store: Arc<S>,
    clock: Arc<dyn Clock>,
    executor: Arc<E>,
    registry: Arc<ToolRegistry>,
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
        }
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
                let actions = project_next_actions(job_id, projection.state);
                Ok(QueryResult::NextActions(actions))
            }
        }
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
            if let Some(fp) = active_candidate_fingerprint.as_ref() {
                if let Ok(r) = self.store.get_gate_result(*gid, fp).await {
                    gates.insert(*gid, r);
                }
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
            resume_event: None,
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

        Ok(CommandResult {
            new_version: 0,
            new_sequence: sequence,
            side_effects: vec![side_effect],
        })
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

        let record = self
            .executor
            .execute(request)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Executor, e.to_string()))?;

        // Translate executor-owned `Outcome` (PHASE-0B.md §32) into
        // the canonical evidence-status vocabulary. The mapping
        // collapses Timeout / Interrupted to Fail (they are tool-side
        // outcomes, not infrastructure problems) and lifts
        // InfrastructureFailed into the dedicated evidence status so
        // gate evaluation can react accordingly.
        let outcome = ironmaint_executor::outcome_from_record(&record);
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
        let producer = EvidenceProducer::new(cap_key.as_str().to_string());
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

/// True if the job state can record evidence from a runtime
/// check. Pre-capture states (EventDetected..CandidateAssembly)
/// have no active candidate fingerprint and cannot materialise
/// checks; the handler rejects them.
fn post_capture_state(state: ironmaint_core::JobState) -> bool {
    use ironmaint_core::JobState as S;
    matches!(
        state,
        S::SourceRevision
            | S::SourceIntegrity
            | S::BuildValidation
            | S::PackageQaValidation
            | S::FunctionalValidation
            | S::UpgradeValidation
            | S::ReleaseReview
            | S::FinalValidation
            | S::ReadyForApproval
            | S::Approved
            | S::PublicationPending
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
            blockers: vec![ActionBlocker::GatePending {
                tool_key: "synthetic.build.validate".to_string(),
            }],
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
