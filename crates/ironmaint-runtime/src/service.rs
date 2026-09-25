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
    MaintenanceJob,
};
use ironmaint_evidence::{Evidence, EvidenceProducer, EvidenceScope};
use ironmaint_executor::{ExecutionRequest, Executor, ToolRegistry};
use ironmaint_state::JobEvent;
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
                tool_key,
                retry_class,
            } => self.handle_run_check(job_id, tool_key, retry_class).await,
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

    /// `RunCheck`: look up a registered tool, invoke it via the
    /// executor, normalise the result, and persist the resulting
    /// evidence. The tool must be registered in the runtime's
    /// `ToolRegistry`; the executor must be configured to run it.
    async fn handle_run_check(
        &self,
        job_id: JobId,
        tool_key: String,
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

        let cap_key = ToolCapabilityKey::new(tool_key.clone()).map_err(|e| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("invalid tool_key {tool_key:?}: {e}"),
            )
        })?;

        let _tool = self.registry.get(&cap_key).ok_or_else(|| {
            RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("no tool registered for capability {tool_key}"),
            )
        })?;

        let request = ExecutionRequest::new(cap_key, retry_class, serde_json::Value::Null);

        let record = self
            .executor
            .execute(request)
            .await
            .map_err(|e| RuntimeError::new(RuntimeErrorKind::Executor, e.to_string()))?;

        // Default exit-code-based classifier: 0 → Pass, anything else → Fail.
        let outcome_status = if record.succeeded() {
            ironmaint_evidence::EvidenceStatus::Pass
        } else {
            ironmaint_evidence::EvidenceStatus::Fail
        };

        let now = self.clock.now_utc();
        let scope = EvidenceScope::Candidate(fingerprint.clone());
        let producer = EvidenceProducer::new(tool_key.clone());
        let evidence = Evidence::new(
            fingerprint,
            ironmaint_evidence::EvidenceKind::Other("tool_output".to_string()),
            outcome_status,
            producer,
            scope,
            now,
        );

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
            side_effects: vec![format!("ran tool {tool_key} on job:{job_id}")],
        })
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
