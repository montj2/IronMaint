//! Adapter error taxonomy (§54).
//!
//! "Never encode expected package build failures as `AdapterError`."
//! A build that doesn't compile is a [`crate::BuildCapability::build_plan`]
//! returning `Fail`-grade gates (or `FailedObligation` via the
//! state machine), not an adapter error. `AdapterError` is reserved
//! for distribution- or provider-specific concerns that prevent the
//! adapter from doing its job.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Adapter error (§54).
///
/// `kind` is the categorical taxonomy; `message` is a free-form
/// human-readable string with no semantic contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdapterError {
    pub kind: AdapterErrorKind,
    pub message: String,
}

impl AdapterError {
    #[must_use]
    pub fn new(kind: AdapterErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "adapter error [{}]: {}", self.kind, self.message)
    }
}

impl std::error::Error for AdapterError {}

/// Categorical adapter error taxonomy (§54).
///
/// Rules:
///
/// - `Unsupported` — capability cannot perform requested operation.
/// - `Invalid*` — package/input problem.
/// - `HumanReviewRequired` — deterministic automation must stop.
/// - `ExternalTransient` — retry may succeed.
/// - `ExternalPermanent` — retry without changed conditions is
///   inappropriate.
/// - `InternalAdapterFailure` — implementation defect or invariant
///   failure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AdapterErrorKind {
    Unsupported,
    InvalidPackage,
    InvalidVersion,
    InvalidConfiguration,
    MissingRequiredMetadata,
    PolicyUnavailable,
    HumanReviewRequired,
    ExternalTransient,
    ExternalPermanent,
    InternalAdapterFailure,
}

impl AdapterErrorKind {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::InvalidPackage => "invalid_package",
            Self::InvalidVersion => "invalid_version",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::MissingRequiredMetadata => "missing_required_metadata",
            Self::PolicyUnavailable => "policy_unavailable",
            Self::HumanReviewRequired => "human_review_required",
            Self::ExternalTransient => "external_transient",
            Self::ExternalPermanent => "external_permanent",
            Self::InternalAdapterFailure => "internal_adapter_failure",
        }
    }
}

impl fmt::Display for AdapterErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&AdapterErrorKind::HumanReviewRequired).unwrap(),
            r#""human_review_required""#
        );
        assert_eq!(
            serde_json::to_string(&AdapterErrorKind::InvalidConfiguration).unwrap(),
            r#""invalid_configuration""#
        );
    }

    #[test]
    fn kind_name_is_snake_case() {
        assert_eq!(
            AdapterErrorKind::ExternalTransient.name(),
            "external_transient"
        );
    }

    #[test]
    fn error_display() {
        let e = AdapterError::new(
            AdapterErrorKind::Unsupported,
            "this distribution does not support that operation",
        );
        let s = e.to_string();
        assert!(s.contains("unsupported"));
        assert!(s.contains("this distribution"));
    }

    #[test]
    fn error_round_trips() {
        let e = AdapterError::new(
            AdapterErrorKind::InvalidVersion,
            "tilde precedence unsupported in stub",
        );
        let json = serde_json::to_string(&e).unwrap();
        let parsed: AdapterError = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, e);
    }
}
