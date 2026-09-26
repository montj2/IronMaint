//! `ironmaint-executor` — execution primitives.
//!
//! PHASE-0B.md §21-§33. The executor is the bridge between the
//! adapter's `PlannedCheck` and the runtime's evidence ledger.
//! Its three responsibilities are:
//!
//! 1. Carry out the actual tool invocation (sandboxed subprocess
//!    or in-process call against a registered capability).
//! 2. Decide whether a failure is *retryable*, and if so, how
//!    many times — three retry classes are fixed by §28:
//!    `Safe`, `Conditional`, `Never`.
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

pub mod artifact_guard;
pub mod env;
pub mod error;
pub mod executor;
pub mod fixture;
pub mod limits;
pub mod normalizer;
pub mod operation;
pub mod process;
pub mod record;
pub mod record_builder;
pub mod registry;
pub mod request;
pub mod retry;

pub use artifact_guard::{
    JobArtifactGuard, JobArtifactGuardFactory, JobArtifactUsage, factory_from_config,
    permissive_guard,
};
pub use env::ProcessEnvironment;
pub use error::{ExecutorError, ExecutorErrorKind};
pub use executor::{Executor, NullExecutor};
pub use fixture::{FixtureNormalizer, Outcome, outcome_from_record};
pub use limits::{ExecutionClass, ExecutionLimits, LimitsConfig};
pub use normalizer::{NormalizationError, NormalizedResult, Observation, ResultNormalizer};
pub use operation::{OperationLifecycle, RecoveryReport};
pub use process::{ProcessExecutor, TimeFactory, wall_clock_time};
pub use record::ExecutionRecord;
pub use record_builder::ToolDefinitionRecord;
pub use registry::{RegistryError, ToolDefinition, ToolRegistry};
pub use request::ExecutionRequest;
pub use retry::RetryClass;
