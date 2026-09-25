//! Operation persistence (`PHASE-0B.md §6, §33`).
//!
//! Each `PrivilegedOperation` represents a single in-flight or
//! completed privileged side-effect. Lifecycle:
//! `Proposed → Authorized → Executing → Succeeded | Failed |
//! Interrupted | InfrastructureFailed`.
//!
//! `Interrupted` is distinct from `Failed`: it indicates the
//! executor didn't get a clean result (crash, SIGKILL, network
//! partition). Recovery logic in the runtime resolves
//! `Interrupted` records on next daemon start.

use ironmaint_core::{JobId, OperationId};
use ironmaint_policy::PrivilegedOperation;

use crate::error::StoreError;

/// Durable storage for [`PrivilegedOperation`] records and their
/// execution state.
pub trait OperationStore: Send + Sync {
    /// Persist a new operation in `Proposed` state. The runtime
    /// calls this when the agent proposes a privileged side-effect.
    /// `job_id` is supplied by the caller (see [`EvidenceStore`]).
    ///
    /// [`EvidenceStore`]: crate::evidence::EvidenceStore
    async fn put_operation(
        &self,
        operation: &PrivilegedOperation,
        job_id: JobId,
    ) -> Result<OperationId, StoreError>;

    /// Look up an operation by id.
    async fn get_operation(&self, id: OperationId) -> Result<PrivilegedOperation, StoreError>;

    /// Persist a state change. Used by the privileged service to
    /// advance `AuthorizationState` and by the executor to record
    /// the final outcome.
    async fn update_operation(
        &self,
        id: OperationId,
        updated: &PrivilegedOperation,
    ) -> Result<(), StoreError>;

    /// List operations currently in `Executing` state. Used at
    /// daemon startup to identify `Interrupted` records for
    /// recovery.
    async fn list_executing_operations(&self) -> Result<Vec<OperationId>, StoreError>;

    /// List operations attached to a job, newest-first.
    async fn list_operations_for_job(&self, job_id: JobId) -> Result<Vec<OperationId>, StoreError>;
}
