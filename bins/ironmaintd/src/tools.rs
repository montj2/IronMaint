//! `ironmaintd::tools` — daemon-side tool registration.
//!
//! PHASE-2 PR 2A.1: the daemon's tool registry is built
//! in two parts. The synthetic tools (the six
//! `synthetic.build.*` / `synthetic.qa.*` / etc. keys)
//! are registered by `register_synthetic_tools` in
//! `ironmaint-synthetic-tools`. The real Debian
//! inspection + build tools (the six
//! `debian.inspect.*` / `debian.policy.delta_review` /
//! `debian.issue.bts_snapshot` / `debian.build.sbuild`
//! keys) are registered by `register_debian_tools` in
//! this module.
//!
//! 2A.1 adds `debian.build.sbuild` as the sixth
//! `debian.*` tool. The hermetic test
//! `bins/ironmaintd/tests/build_tool_registration.rs`
//! calls `register_debian_tools` directly to assert
//! the key is registered.
//!
//! ## Why a helper, not in-line `tools.register(...)` calls
//!
//! The pre-2A.1 daemon's `main.rs` registered each
//! Debian tool with its own in-line
//! `ToolDefinitionRecord::new(...)` block
//! (`bins/ironmaintd/src/main.rs:227-388`). The in-line
//! form made the 1C.x / 1D.2 / 1E.2 PRs each
//! self-contained: one block per PR, one
//! `register(...)` call, one `tracing::info!` line.
//! 2A.1 extracts the in-line form into a helper so
//! the new `debian.build.sbuild` registration is
//! visible in the same place as the existing five and
//! so a hermetic test can call the helper to assert
//! the key is present without spinning up the full
//! daemon.

use std::collections::BTreeMap;
use std::path::Path;

use ironmaint_executor::{RegistryError, ToolRegistry};
use ironmaint_executor::{ToolDefinitionRecord, ToolInputMode};

/// Register the six real Debian tools in `tools`.
///
/// The six tools:
///
/// - `debian.inspect.source_preparation` (1C.1)
/// - `debian.inspect.source_analysis` (1C.2)
/// - `debian.inspect.maintenance_context` (1C.3)
/// - `debian.policy.delta_review` (1D.2)
/// - `debian.issue.bts_snapshot` (1E.2)
/// - `debian.build.sbuild` (2A.1)
///
/// `debian_tool_bin` is the path to the
/// `ironmaint-debian-tool` binary; the helper uses it
/// as the `executable` for all six. `bts_helper` is the
/// path to the `bts-helper.py` shim, used as the
/// `BTS_HELPER` env var for the 1E.2 tool; `None`
/// means the operator has not configured one and the
/// bts-snapshot tool's env is empty.
pub fn register_debian_tools(
    tools: &mut ToolRegistry,
    debian_tool_bin: &Path,
    bts_helper: Option<&Path>,
) -> Result<(), RegistryError> {
    register_debian_inspect_source_preparation(tools, debian_tool_bin)?;
    register_debian_inspect_source_analysis(tools, debian_tool_bin)?;
    register_debian_inspect_maintenance_context(tools, debian_tool_bin)?;
    register_debian_policy_delta_review(tools, debian_tool_bin)?;
    register_debian_issue_bts_snapshot(tools, debian_tool_bin, bts_helper)?;
    register_debian_build_sbuild(tools, debian_tool_bin)?;
    Ok(())
}

fn debian_limits_30s() -> ironmaint_executor::ExecutionLimits {
    ironmaint_executor::ExecutionLimits {
        timeout: std::time::Duration::from_secs(30),
        stdout_max_bytes: 256 * 1024,
        stderr_max_bytes: 64 * 1024,
    }
}

fn debian_limits_build() -> ironmaint_executor::ExecutionLimits {
    // Conservative 2A.1 defaults. 2A.3 tunes these
    // against the `pinned-real` fixture once the real
    // sbuild invocation is in place. 30 minutes is the
    // Debian sbuild default timeout; 8 MiB / 8 MiB
    // gives the build log + sbuild metadata room to
    // grow without truncation under §15 retention.
    ironmaint_executor::ExecutionLimits {
        timeout: std::time::Duration::from_secs(30 * 60),
        stdout_max_bytes: 8 * 1024 * 1024,
        stderr_max_bytes: 8 * 1024 * 1024,
    }
}

fn register_debian_inspect_source_preparation(
    tools: &mut ToolRegistry,
    debian_tool_bin: &Path,
) -> Result<(), RegistryError> {
    let key = ironmaint_adapter_api::ToolCapabilityKey::new("debian.inspect.source_preparation")
        .map_err(|e| RegistryError::AlreadyRegistered(e.to_string()))?;
    let record = ToolDefinitionRecord::new(
        key,
        debian_tool_bin,
        vec![std::ffi::OsString::from("source-preparation")],
        ironmaint_executor::ExecutionClass::Check,
        debian_limits_30s(),
    )
    .with_input_mode(ToolInputMode::JsonStdin)
    .with_normalizer(std::sync::Arc::new(
        ironmaint_debian_tool::DebianSourcePreparationNormalizer::new(),
    ));
    tools.register(Box::new(record))
}

fn register_debian_inspect_source_analysis(
    tools: &mut ToolRegistry,
    debian_tool_bin: &Path,
) -> Result<(), RegistryError> {
    let key = ironmaint_adapter_api::ToolCapabilityKey::new("debian.inspect.source_analysis")
        .map_err(|e| RegistryError::AlreadyRegistered(e.to_string()))?;
    let record = ToolDefinitionRecord::new(
        key,
        debian_tool_bin,
        vec![std::ffi::OsString::from("source-analysis")],
        ironmaint_executor::ExecutionClass::Check,
        debian_limits_30s(),
    )
    .with_input_mode(ToolInputMode::JsonStdin)
    .with_normalizer(std::sync::Arc::new(
        ironmaint_debian_tool::DebianSourceAnalysisNormalizer::new(),
    ));
    tools.register(Box::new(record))
}

fn register_debian_inspect_maintenance_context(
    tools: &mut ToolRegistry,
    debian_tool_bin: &Path,
) -> Result<(), RegistryError> {
    let key = ironmaint_adapter_api::ToolCapabilityKey::new("debian.inspect.maintenance_context")
        .map_err(|e| RegistryError::AlreadyRegistered(e.to_string()))?;
    let record = ToolDefinitionRecord::new(
        key,
        debian_tool_bin,
        vec![std::ffi::OsString::from("maintenance-context")],
        ironmaint_executor::ExecutionClass::Check,
        debian_limits_30s(),
    )
    .with_input_mode(ToolInputMode::JsonStdin)
    .with_normalizer(std::sync::Arc::new(
        ironmaint_debian_tool::DebianMaintenanceContextNormalizer::new(),
    ));
    tools.register(Box::new(record))
}

fn register_debian_policy_delta_review(
    tools: &mut ToolRegistry,
    debian_tool_bin: &Path,
) -> Result<(), RegistryError> {
    let key = ironmaint_adapter_api::ToolCapabilityKey::new("debian.policy.delta_review")
        .map_err(|e| RegistryError::AlreadyRegistered(e.to_string()))?;
    let record = ToolDefinitionRecord::new(
        key,
        debian_tool_bin,
        vec![std::ffi::OsString::from("policy-delta")],
        ironmaint_executor::ExecutionClass::Check,
        debian_limits_30s(),
    )
    .with_input_mode(ToolInputMode::JsonStdin)
    .with_normalizer(std::sync::Arc::new(
        ironmaint_debian_tool::DebianPolicyDeltaReviewNormalizer::new(),
    ));
    tools.register(Box::new(record))
}

fn register_debian_issue_bts_snapshot(
    tools: &mut ToolRegistry,
    debian_tool_bin: &Path,
    bts_helper: Option<&Path>,
) -> Result<(), RegistryError> {
    let key = ironmaint_adapter_api::ToolCapabilityKey::new("debian.issue.bts_snapshot")
        .map_err(|e| RegistryError::AlreadyRegistered(e.to_string()))?;
    // PHASE-1.1 PR 1.1.I: when the operator has
    // configured a helper path (via `--bts-helper`
    // or `IRONMAINT_BTS_HELPER`), inject it as the
    // bts-snapshot tool's per-tool env. When unset,
    // the tool's env is empty and the subcommand's
    // hermetic fallback fires (no `BTS_HELPER` in
    // the now-closed env).
    let bts_env: BTreeMap<String, String> = match bts_helper {
        Some(path) => {
            let mut env = BTreeMap::new();
            env.insert(
                "BTS_HELPER".to_string(),
                path.to_string_lossy().into_owned(),
            );
            env
        }
        None => BTreeMap::new(),
    };
    let record = ToolDefinitionRecord::new(
        key,
        debian_tool_bin,
        vec![std::ffi::OsString::from("bts-snapshot")],
        ironmaint_executor::ExecutionClass::Check,
        debian_limits_30s(),
    )
    .with_input_mode(ToolInputMode::JsonStdin)
    .with_env(bts_env)
    .with_normalizer(std::sync::Arc::new(
        ironmaint_debian_tool::DebianBtsSnapshotNormalizer::new(),
    ));
    tools.register(Box::new(record))
}

/// PHASE-2 PR 2A.1. Register `debian.build.sbuild`.
///
/// The 2A.1 build tool: `ironmaint-debian-tool` with
/// the `build` subcommand. Conservative 30-minute
/// timeout + 8 MiB stdout / 8 MiB stderr limits; 2A.3
/// tunes these against the `pinned-real` fixture once
/// the real sbuild invocation is in place. The
/// `BUILD_*` env vars (per-call) are injected by the
/// runtime in 2A.2 via `ExecutionRequest::env_overrides`;
/// the tool's own `env()` is empty in 2A.1.
fn register_debian_build_sbuild(
    tools: &mut ToolRegistry,
    debian_tool_bin: &Path,
) -> Result<(), RegistryError> {
    let key = ironmaint_adapter_api::ToolCapabilityKey::new("debian.build.sbuild")
        .map_err(|e| RegistryError::AlreadyRegistered(e.to_string()))?;
    let record = ToolDefinitionRecord::new(
        key,
        debian_tool_bin,
        vec![std::ffi::OsString::from("build")],
        ironmaint_executor::ExecutionClass::Check,
        debian_limits_build(),
    )
    .with_input_mode(ToolInputMode::JsonStdin)
    .with_normalizer(std::sync::Arc::new(
        ironmaint_debian_tool::DebianBuildReportNormalizer::new(),
    ));
    tools.register(Box::new(record))
}
