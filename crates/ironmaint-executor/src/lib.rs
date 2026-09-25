//! `ironmaint-executor` — execution primitives.
//!
//! PHASE-0B.md §21-§33. The executor is the bridge between the
//! adapter's `PlannedCheck` and the runtime's evidence ledger.
//! Its three responsibilities are:
//!
//! 1. Carry out the actual tool invocation (sandboxed subprocess
//!    or in-process call against a registered capability).
//! 2. Decide whether a failure is *retryable*, and if so, how
//!    many times — three retry classes are fixed by §27:
//!    `Idempotent`, `SideEffecting`, `Destructive`.
//! 3. Materialise the result into a `NormalizedResult` that the
//!    runtime can persist as evidence.
//!
//! The executor is async (native `async fn` in traits; MSRV 1.85)
//! because subprocess I/O is async. The traits it consumes
//! (`ResultNormalizer`, `ToolDefinition`) are sync — the executor
//! itself bridges them.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
    )
)]

pub mod env;
pub mod error;
pub mod executor;
pub mod record;
pub mod request;
pub mod retry;

pub use env::ProcessEnvironment;
pub use error::{ExecutorError, ExecutorErrorKind};
pub use executor::{Executor, NullExecutor};
pub use record::ExecutionRecord;
pub use request::ExecutionRequest;
pub use retry::RetryClass;
