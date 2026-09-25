//! `RuntimeService` — façade that ties the store, the state
//! engine, the executor, and the workspace into a single
//! command/query API.
//!
//! The daemon (commit 14) constructs one of these on startup
//! and hands it to the MCP server. Commit 13 ships a
//! minimum-viable shape: the dispatcher recognises every
//! command/query enum variant, but heavy lifting (running an
//! executor subprocess, applying a patch) lands in later
//! commits when those pieces are wired through.

use std::sync::Arc;

use ironmaint_core::JobId;
use ironmaint_store::IronMaintStore;

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

pub struct RuntimeService<S: IronMaintStore + ?Sized> {
    store: Arc<S>,
}

impl<S: IronMaintStore + ?Sized> std::fmt::Debug for RuntimeService<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeService").finish()
    }
}

impl<S: IronMaintStore + ?Sized> RuntimeService<S> {
    #[must_use]
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    pub async fn handle_command(&self, cmd: RuntimeCommand) -> Result<CommandResult, RuntimeError> {
        match cmd {
            RuntimeCommand::CreateJob {
                orchestrator: _,
                package: _,
            } => Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                "CreateJob not wired in commit 13; daemon wires it in 0B.14",
            )),
            RuntimeCommand::SetActiveCandidate {
                job_id: _,
                fingerprint: _,
            } => Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                "SetActiveCandidate not wired in commit 13; runtime wires it in 0B.13-followups",
            )),
            RuntimeCommand::RecordCheckEvidence { job_id, .. } => Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("RecordCheckEvidence for {job_id} not yet implemented"),
            )),
            RuntimeCommand::MarkObligationSatisfied { job_id, .. } => Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("MarkObligationSatisfied for {job_id} not yet implemented"),
            )),
            RuntimeCommand::RunCheck { job_id, .. } => Err(RuntimeError::new(
                RuntimeErrorKind::InvalidInput,
                format!("RunCheck for {job_id} not yet implemented"),
            )),
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
fn _assert_service_send_sync<S: IronMaintStore + Send + Sync + ?Sized>() {
    _assert_send_sync::<JobId>();
    _assert_send_sync::<RuntimeService<S>>();
}
