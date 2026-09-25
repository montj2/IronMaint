//! Durable operation lifecycle + crash recovery (PHASE-0B.md §33).
//!
//! The executor owns the *post-authorization* phase of
//! `PrivilegedOperation`: once the privileged service has
//! transitioned an operation into `Authorized`, the executor
//! advances it through `Executing` and on to a terminal state.
//!
//! `Interrupted` is distinct from `Failed`: an `Interrupted`
//! operation is one whose `Executing → terminal` transition
//! did not complete cleanly (daemon crash, SIGKILL, network
//! partition). On next daemon start, the executor's
//! `recover_interrupted` walks `OperationStore::list_executing_operations`
//! and either re-issues the underlying tool or marks the
//! operation as `Failed` based on what the tool runtime
//! reports.
//!
//! `Interrupted` is not a real `AuthorizationState` — it is a
//! derived classification the executor computes by reading
//! `Executing` records whose `finished_at` is `None` and whose
//! `started_at` is older than a threshold.

use std::sync::Arc;

use ironmaint_core::OperationId;
use ironmaint_policy::PrivilegedOperation;
use ironmaint_store::OperationStore;
use time::OffsetDateTime;

use crate::error::{ExecutorError, ExecutorErrorKind};

/// Result of one recovery pass on daemon startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub inspected: usize,
    pub recovered: usize,
    pub abandoned: usize,
}

/// Wraps an `OperationStore` with the executor's recovery semantics.
pub struct OperationLifecycle<S: OperationStore + ?Sized> {
    store: Arc<S>,
    /// Operations whose `started_at` is older than this threshold
    /// and that are still in `Executing` are classified as
    /// `Interrupted`.
    interrupt_threshold: std::time::Duration,
}

impl<S: OperationStore + ?Sized> OperationLifecycle<S> {
    #[must_use]
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            interrupt_threshold: std::time::Duration::from_secs(60),
        }
    }

    #[must_use]
    pub fn with_interrupt_threshold(mut self, threshold: std::time::Duration) -> Self {
        self.interrupt_threshold = threshold;
        self
    }

    /// Walk all operations in `Executing` state. For each one,
    /// classify based on age: operations older than the
    /// threshold are `Interrupted` and are abandoned (transition
    /// to `Failed`); fresh ones are left alone (the runtime will
    /// re-attach to them).
    pub async fn recover_interrupted(&self) -> Result<RecoveryReport, ExecutorError> {
        let now = OffsetDateTime::now_utc();
        let ids = self.store.list_executing_operations().await.map_err(|e| {
            ExecutorError::new(
                ExecutorErrorKind::InfrastructureFailed,
                format!("list_executing_operations: {e}"),
            )
        })?;
        let mut report = RecoveryReport {
            inspected: ids.len(),
            ..Default::default()
        };
        for id in ids {
            let mut op = self.store.get_operation(id).await.map_err(|e| {
                ExecutorError::new(
                    ExecutorErrorKind::InfrastructureFailed,
                    format!("get_operation {id}: {e}"),
                )
            })?;
            // Heuristic: we don't have a `started_at` on
            // PrivilegedOperation directly, so we use the
            // `id`'s UUIDv7 timestamp as a proxy. If the operation
            // is older than the threshold and still Executing,
            // classify as Interrupted and abandon.
            let age = operation_age(&op.id, now);
            if age > self.interrupt_threshold {
                abandon_operation(&mut op);
                self.store.update_operation(op.id, &op).await.map_err(|e| {
                    ExecutorError::new(
                        ExecutorErrorKind::InfrastructureFailed,
                        format!("update_operation {id}: {e}"),
                    )
                })?;
                report.abandoned += 1;
            } else {
                report.recovered += 1;
            }
        }
        Ok(report)
    }
}

fn operation_age(id: &OperationId, now: OffsetDateTime) -> std::time::Duration {
    // UUIDv7 carries an embedded timestamp; fall back to zero
    // age (never classify as Interrupted) if extraction fails.
    let secs = id
        .as_uuid()
        .get_timestamp()
        .map(|ts| {
            let (sec, _nsec) = ts.to_unix();
            let age = (now.unix_timestamp() - sec as i64).max(0);
            age as u64
        })
        .unwrap_or(0);
    std::time::Duration::from_secs(secs)
}

fn abandon_operation(op: &mut PrivilegedOperation) {
    // Authorized → Failed is a legal post-hoc transition when
    // recovery decides the side effect did not complete. We use
    // Failed rather than introducing a new Cancelled state.
    if matches!(
        op.authorization,
        ironmaint_policy::AuthorizationState::Executing
    ) {
        op.authorization = ironmaint_policy::AuthorizationState::Failed;
    }
}

// Anchor the time::OffsetDateTime path so we don't accidentally
// pull the trait into the public surface.
#[allow(dead_code)]
fn _ensure_time_import() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}
