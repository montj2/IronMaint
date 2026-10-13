//! Synthetic tool definitions — Phase 0B.9.
//!
//! The six `synthetic.build.*` tool definitions used by the
//! conformance suite (PHASE-0B.md §92) and by the daemon's tool
//! registry live here so there is exactly one source of truth for
//! them.
//!
//! This crate exists for two reasons:
//!
//! 1. `ironmaint-testkit` is a conformance runner, not a
//!    production dependency (PHASE-0A.md §67, enforced by
//!    `xtask verify-architecture`). The daemon therefore cannot
//!    borrow the definitions from where they used to live.
//! 2. The daemon must not serve `check.run` against an empty
//!    registry. Registering the synthetic tools is what lets the
//!    Phase 0B fixture workflow actually execute a check end to
//!    end.
//!
//! These are *definitions*, not behaviour: the real logic lives in
//! the `ironmaint-fixture` binary they point at, and Phase 1
//! replaces this whole crate with genuine adapter plans.
//!
//! [Phase 0B.9]: ../../doc/PHASE-0B-COMPLETION.md

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

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_executor::{ExecutionClass, ExecutionLimits, ToolDefinitionRecord, ToolRegistry};

/// Failure to define the synthetic tool set.
#[derive(Debug, thiserror::Error)]
pub enum SyntheticToolError {
    /// A key failed the `<namespace>.<role>.<tool>` shape check.
    #[error("invalid tool capability key: {0}")]
    InvalidKey(String),
    /// The registry rejected a [`ToolDefinitionRecord`].
    #[error("tool registration failed: {0}")]
    Registration(String),
}

/// The six synthetic tool keys, returned by
/// [`register_synthetic_tools`].
///
/// Callers pass this to the conformance suite so it exercises the
/// same fixture binary the harness registered.
#[derive(Debug, Clone)]
pub struct SyntheticKeys {
    /// Succeeds with exit 0 and clean output.
    pub validate: ToolCapabilityKey,
    /// Fails with exit 1 — a tool failure, not an infra failure.
    pub fail: ToolCapabilityKey,
    /// Sleeps past its 500ms wall-clock cap.
    pub timeout: ToolCapabilityKey,
    /// Emits 256 KiB against a 4 KiB stdout cap.
    pub truncate: ToolCapabilityKey,
    /// Exits on SIGTERM, exercising interruption recovery.
    pub interrupt: ToolCapabilityKey,
    /// Exits with the infra-failure marker.
    pub infra_fail: ToolCapabilityKey,
}

/// Register the six synthetic tool keys against `registry`, all
/// pointing at `fixture_bin`. The fixture's argv interface takes the
/// key as its first positional argument.
///
/// Per-key limits:
///
/// - `validate`, `fail`, `interrupt`, `infra_fail` — defaults.
/// - `timeout` — 500ms wall-clock cap (the fixture sleeps far longer).
/// - `truncate` — 4 KiB stdout cap (the fixture emits 256 KiB).
///
/// # Errors
///
/// Returns [`SyntheticToolError::InvalidKey`] if a key fails shape
/// validation, or [`SyntheticToolError::Registration`] if the
/// registry rejects a record (for example, on a duplicate key).
pub fn register_synthetic_tools(
    registry: &mut ToolRegistry,
    fixture_bin: &Path,
) -> Result<SyntheticKeys, SyntheticToolError> {
    let default_limits = ExecutionLimits::default();
    let timeout_limits = ExecutionLimits {
        timeout: Duration::from_millis(500),
        ..default_limits.clone()
    };
    let truncate_limits = ExecutionLimits {
        timeout: Duration::from_secs(5),
        stdout_max_bytes: 4 * 1024,
        stderr_max_bytes: 1024,
    };

    let validate = register_one(
        registry,
        "synthetic.build.validate",
        default_limits.clone(),
        fixture_bin,
    )?;
    let fail = register_one(
        registry,
        "synthetic.build.fail",
        default_limits.clone(),
        fixture_bin,
    )?;
    let timeout = register_one(
        registry,
        "synthetic.build.timeout",
        timeout_limits,
        fixture_bin,
    )?;
    let truncate = register_one(
        registry,
        "synthetic.build.truncate",
        truncate_limits,
        fixture_bin,
    )?;
    let interrupt = register_one(
        registry,
        "synthetic.build.interrupt",
        default_limits.clone(),
        fixture_bin,
    )?;
    let infra_fail = register_one(
        registry,
        "synthetic.build.infra_fail",
        default_limits,
        fixture_bin,
    )?;

    Ok(SyntheticKeys {
        validate,
        fail,
        timeout,
        truncate,
        interrupt,
        infra_fail,
    })
}

/// Define one synthetic tool and install it in `registry`.
fn register_one(
    registry: &mut ToolRegistry,
    key: &str,
    limits: ExecutionLimits,
    fixture_bin: &Path,
) -> Result<ToolCapabilityKey, SyntheticToolError> {
    let capability = ToolCapabilityKey::new(key)
        .map_err(|e| SyntheticToolError::InvalidKey(format!("{key}: {e}")))?;
    let record = ToolDefinitionRecord::new(
        capability.clone(),
        fixture_bin,
        vec![OsString::from(key)],
        ExecutionClass::Check,
        limits,
    );
    registry
        .register(Box::new(record))
        .map_err(|e| SyntheticToolError::Registration(format!("{key}: {e}")))?;
    Ok(capability)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_bin() -> PathBuf {
        PathBuf::from("/usr/bin/ironmaint-fixture")
    }

    #[test]
    fn registers_all_six_keys() {
        let mut registry = ToolRegistry::new();
        let keys = register_synthetic_tools(&mut registry, &fixture_bin())
            .expect("six synthetic keys are well-formed");

        assert_eq!(keys.validate.as_str(), "synthetic.build.validate");
        assert_eq!(keys.fail.as_str(), "synthetic.build.fail");
        assert_eq!(keys.timeout.as_str(), "synthetic.build.timeout");
        assert_eq!(keys.truncate.as_str(), "synthetic.build.truncate");
        assert_eq!(keys.interrupt.as_str(), "synthetic.build.interrupt");
        assert_eq!(keys.infra_fail.as_str(), "synthetic.build.infra_fail");
    }

    #[test]
    fn every_key_is_resolvable_from_the_registry() {
        let mut registry = ToolRegistry::new();
        let keys =
            register_synthetic_tools(&mut registry, &fixture_bin()).expect("registration succeeds");

        for key in [
            &keys.validate,
            &keys.fail,
            &keys.timeout,
            &keys.truncate,
            &keys.interrupt,
            &keys.infra_fail,
        ] {
            assert!(
                registry.get(key).is_some(),
                "{key} must resolve from the registry"
            );
        }
    }

    #[test]
    fn timeout_key_gets_the_short_wall_clock_cap() {
        let mut registry = ToolRegistry::new();
        let keys =
            register_synthetic_tools(&mut registry, &fixture_bin()).expect("registration succeeds");

        let record = registry.get(&keys.timeout).expect("timeout key registered");
        assert_eq!(record.limits().timeout, Duration::from_millis(500));
    }

    #[test]
    fn truncate_key_gets_the_small_stdout_cap() {
        let mut registry = ToolRegistry::new();
        let keys =
            register_synthetic_tools(&mut registry, &fixture_bin()).expect("registration succeeds");

        let record = registry
            .get(&keys.truncate)
            .expect("truncate key registered");
        assert_eq!(record.limits().stdout_max_bytes, 4 * 1024);
    }

    #[test]
    fn duplicate_registration_is_reported_not_panicked() {
        // The pre-0B.9 implementation used `expect` here, so a
        // second registration would abort the process. Production
        // crates deny `expect_used`, and a daemon must fail
        // recoverably rather than panic on a config mistake.
        let mut registry = ToolRegistry::new();
        register_synthetic_tools(&mut registry, &fixture_bin())
            .expect("first registration succeeds");

        let second = register_synthetic_tools(&mut registry, &fixture_bin());
        assert!(
            matches!(second, Err(SyntheticToolError::Registration(_))),
            "second registration must be a typed error, got {second:?}"
        );
    }
}
