//! Runtime configuration.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ironmaint_executor::{ExecutionLimits, JobArtifactGuardFactory, permissive_guard};

/// Runtime configuration surface. Consumed by the daemon entry
/// point (PHASE-0B.md §53); carried as data so daemon wiring does
/// not need to revisit the struct on every executor-side change.
///
/// `artifact_guard_factory` is a function-trait object that cannot
/// be serialised or formatted — closures cannot round-trip through
/// JSON, and the trait object has no `Debug` impl. The field is
/// `#[serde(skip)]` with a runtime default; configuration files
/// describe limits, address, and state directory, not per-process
/// closures. `Debug` is implemented manually for the same reason.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeConfig {
    pub state_dir: PathBuf,
    pub listen_addr: SocketAddr,
    pub log_level: String,
    /// Default per-tool execution limits (timeout, stdout / stderr
    /// byte caps) applied to any tool registered without an
    /// explicit override (PHASE-0B.md §31).
    pub executor_limits: ExecutionLimits,
    /// Factory the runtime uses to construct a per-job
    /// `JobArtifactGuard`. Tests inject tighter caps; production
    /// uses the permissive default.
    #[serde(skip, default = "default_guard_factory")]
    #[schemars(skip)]
    pub artifact_guard_factory: JobArtifactGuardFactory,
}

fn default_guard_factory() -> JobArtifactGuardFactory {
    Arc::new(permissive_guard)
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("state_dir", &self.state_dir)
            .field("listen_addr", &self.listen_addr)
            .field("log_level", &self.log_level)
            .field("executor_limits", &self.executor_limits)
            .field(
                "artifact_guard_factory",
                &"<fn(JobId) -> Arc<JobArtifactGuard>>",
            )
            .finish()
    }
}

impl RuntimeConfig {
    #[must_use]
    pub fn minimal(state_dir: PathBuf) -> Self {
        // "127.0.0.1:0" is a syntactically valid SocketAddr; the
        // parse can only fail if the literal is changed. We use
        // unwrap_or with a loopback default to honour the
        // panic-policy lints.
        let listen_addr = "127.0.0.1:0"
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0)));
        Self {
            state_dir,
            listen_addr,
            log_level: "info".to_string(),
            executor_limits: ExecutionLimits::default(),
            artifact_guard_factory: default_guard_factory(),
        }
    }
}
