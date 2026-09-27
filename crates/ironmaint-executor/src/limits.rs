//! Execution class + execution limits (PHASE-0B.md §27, §31).
//!
//! `ExecutionClass` categorises a tool for the executor's spawn path.
//! `ExecutionLimits` carries the per-tool limits that the executor
//! enforces (timeout + stdout/stderr byte caps). `LimitsConfig` carries
//! the per-job and per-artifact counter caps that `JobArtifactGuard`
//! enforces (defined in a later commit but its config lives here so the
//! configuration surface is co-located).

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Category of a registered tool. Controls how the executor spawns the
/// subprocess and which env-var surface it exposes.
///
/// Initial classes (§27): `Check`, `WorkspaceInternal`.
/// `PrivilegedExternal` is reserved for future release-pipeline tools
/// (signing, canonical push). The variant exists so conformance suites
/// can assert the full taxonomy; using it at runtime is rejected by
/// `ProcessExecutor` with a complete error path (no stub).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    /// Read-only check: lintian, rpmlint, sbuild (in check-only mode).
    /// May run in parallel with other checks.
    Check,
    /// Workspace-internal mutation: capture, activation, fixture rewind.
    /// Runs against the per-job workspace; never against upstream.
    WorkspaceInternal,
    /// Reserved for future release-pipeline tools (signing, canonical
    /// push, distribution upload). Not yet implemented in 0B.4.
    PrivilegedExternal,
}

impl ExecutionClass {
    /// Stable wire-format name (snake_case).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::WorkspaceInternal => "workspace_internal",
            Self::PrivilegedExternal => "privileged_external",
        }
    }
}

/// Per-tool execution limits enforced by `ProcessExecutor` (PHASE-0B.md
/// §31). Memory/CPU/PID/disk/network limits are explicitly NOT claimed
/// enforced — spec reserves those for a future sub-phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionLimits {
    /// Wall-clock budget for the subprocess. On expiry the executor
    /// kills the child and returns `ExecutorErrorKind::ToolFailed { timed_out: true }`.
    #[serde(with = "duration_serde")]
    #[schemars(with = "u64")]
    pub timeout: Duration,
    /// Maximum bytes captured from stdout. Output exceeding this cap is
    /// truncated and `ExecutionRecord.truncated` is set.
    pub stdout_max_bytes: u64,
    /// Maximum bytes captured from stderr. Same truncation rule as stdout.
    pub stderr_max_bytes: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            stdout_max_bytes: 256 * 1024,
            stderr_max_bytes: 64 * 1024,
        }
    }
}

/// Per-job aggregate caps (PHASE-0B.md §15 "maximum artifacts per
/// execution", "maximum retained artifact bytes per job") and the
/// per-artifact write cap that mirrors `ArtifactStore::max_artifact_bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LimitsConfig {
    /// Hard ceiling on artifacts written by a single tool run.
    pub per_job_max_artifacts: u32,
    /// Hard ceiling on bytes retained across all artifacts written by a
    /// single tool run.
    pub per_job_max_bytes: u64,
    /// Per-write cap. Mirrors `ArtifactStore::max_artifact_bytes` so
    /// the executor and the blob store agree.
    pub per_artifact_max_bytes: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            per_job_max_artifacts: 1024,
            per_job_max_bytes: 1024 * 1024 * 1024,
            per_artifact_max_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Serde adapter for `Duration` — wire format is integer milliseconds.
mod duration_serde {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        let ms = d.as_millis().min(u64::MAX as u128) as u64;
        ms.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_default_have_spec_values() {
        let l = ExecutionLimits::default();
        assert_eq!(l.timeout, Duration::from_secs(60));
        assert_eq!(l.stdout_max_bytes, 256 * 1024);
        assert_eq!(l.stderr_max_bytes, 64 * 1024);
    }

    #[test]
    fn config_default_matches_artifact_store_cap() {
        let c = LimitsConfig::default();
        assert_eq!(c.per_job_max_artifacts, 1024);
        assert_eq!(c.per_job_max_bytes, 1024 * 1024 * 1024);
        // Mirrors `ArtifactStore::max_artifact_bytes` default.
        assert_eq!(c.per_artifact_max_bytes, 256 * 1024 * 1024);
    }

    #[test]
    fn execution_class_round_trip_via_json() {
        for c in [
            ExecutionClass::Check,
            ExecutionClass::WorkspaceInternal,
            ExecutionClass::PrivilegedExternal,
        ] {
            let json = serde_json::to_string(&c).unwrap();
            let back: ExecutionClass = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn execution_class_wire_format_is_snake_case() {
        assert_eq!(ExecutionClass::Check.as_str(), "check");
        assert_eq!(
            ExecutionClass::WorkspaceInternal.as_str(),
            "workspace_internal"
        );
        assert_eq!(
            ExecutionClass::PrivilegedExternal.as_str(),
            "privileged_external"
        );
    }

    #[test]
    fn execution_limits_serde_round_trip() {
        let original = ExecutionLimits {
            timeout: Duration::from_secs(30),
            stdout_max_bytes: 4096,
            stderr_max_bytes: 2048,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: ExecutionLimits = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn execution_limits_timeout_serialises_as_milliseconds() {
        let l = ExecutionLimits {
            timeout: Duration::from_millis(2500),
            stdout_max_bytes: 0,
            stderr_max_bytes: 0,
        };
        let v: serde_json::Value = serde_json::to_value(&l).unwrap();
        assert_eq!(v["timeout"], serde_json::json!(2500));
    }

    #[test]
    fn execution_class_privileged_external_is_a_real_variant() {
        // No todo!()/unimplemented!() anywhere; the variant is fully
        // constructed and pattern-matchable. Using it at runtime is
        // rejected by ProcessExecutor with a complete error path.
        let c = ExecutionClass::PrivilegedExternal;
        assert_eq!(c, ExecutionClass::PrivilegedExternal);
        assert_ne!(c, ExecutionClass::Check);
    }
}
