//! Executor skeleton (PHASE-0B.md §21).
//!
//! Commit 9 lands the trait and a no-op default impl. Real
//! `ToolRegistry`-backed execution lands in commit 10 (registry
//! + normalizer trait) and commit 12 (fixture binary).

use crate::error::ExecutorError;
use crate::record::ExecutionRecord;
use crate::request::ExecutionRequest;

/// Executor — runs an `ExecutionRequest` and returns a durable
/// `ExecutionRecord`. Native `async fn` in traits (MSRV 1.85).
///
/// Implementations MUST classify failures into
/// `ExecutorErrorKind::ToolFailed` (the tool returned a
/// non-zero exit) or `ExecutorErrorKind::InfrastructureFailed`
/// (the subprocess didn't start, the worker died, the DB is
/// down). Conflating the two is a §33 spec violation.
pub trait Executor: Send + Sync {
    fn execute(
        &self,
        request: ExecutionRequest,
    ) -> impl std::future::Future<Output = Result<ExecutionRecord, ExecutorError>> + Send;
}

/// No-op executor: rejects every request. Used as the default
/// when no real `ToolRegistry` has been wired up yet.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullExecutor;

impl Executor for NullExecutor {
    async fn execute(&self, _request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        Err(ExecutorError::new(
            crate::error::ExecutorErrorKind::UnknownCapability,
            "NullExecutor is the placeholder; replace with a real executor",
        ))
    }
}
