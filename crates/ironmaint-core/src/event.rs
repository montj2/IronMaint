//! Maintenance events (§14).
//!
//! A `MaintenanceEvent` is the trigger that opens a [`MaintenanceJob`]
//! (or, in later phases, a follow-on event mid-job). Event types are
//! distribution-neutral — no Debian/Fedora semantics are encoded in
//! the variant names.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::{CoreError, CoreErrorKind};
use crate::identity::MaintenanceEventId;
use crate::package::PackageIdentity;

const EVENT_SOURCE_MAX: usize = 256;
const EXTERNAL_REFERENCE_MAX: usize = 1024;

/// Categorical kind of a `MaintenanceEvent`.
///
/// Variants deliberately include no Debian/Fedora-specific language.
/// "UpstreamRelease" is intentionally distribution-neutral: a new
/// upstream release triggers the same kind of work whether the
/// package is in Debian, Fedora, or anywhere else.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceEventType {
    UpstreamRelease,
    IssueReported,
    BuildFailure,
    TestFailure,
    PolicyChange,
    SecurityAdvisory,
    DependencyTransition,
    ReleaseTransition,
    ManualRequest,
    /// Catch-all for events that don't fit the typed variants. The
    /// inner string is opaque to core; the adapter or operator decides
    /// how to interpret it.
    Other(String),
}

/// Provenance of a `MaintenanceEvent` — e.g. `upstream-watch`,
/// `distro-bts-poll`, `manual-trigger`.
///
/// Free-form string with the same validation as [`crate::package::PackageName`]:
/// non-empty, no control characters, ≤ 256 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct EventSource(String);

impl EventSource {
    /// Construct an `EventSource`.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidEventSource, .. }` if the
    /// source string is empty, too long, or contains control
    /// characters.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let s = value.into();
        validate_source_string("event source", &s)?;
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for EventSource {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for EventSource {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// A maintenance event: one observation that may initiate or modify a
/// [`MaintenanceJob`](crate::job::MaintenanceJob).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceEvent {
    pub id: MaintenanceEventId,
    pub package: PackageIdentity,
    pub event_type: MaintenanceEventType,
    pub source: EventSource,
    /// RFC3339 UTC. The `time` crate serializer emits a normalized
    /// `+00:00` offset.
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "crate::json_schema_impls::Rfc3339DateTime")]
    pub observed_at: OffsetDateTime,
    /// Optional external-system reference (e.g. a BTS bug number,
    /// Bugzilla id, GitHub issue id). When present, validated
    /// non-empty and within length bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_reference: Option<String>,
}

impl MaintenanceEvent {
    /// Construct a `MaintenanceEvent`.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidEventSource, .. }` if the
    /// source is invalid, or `kind: InvalidName` if the optional
    /// external reference is empty when present.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: MaintenanceEventId,
        package: PackageIdentity,
        event_type: MaintenanceEventType,
        source: EventSource,
        observed_at: OffsetDateTime,
        external_reference: Option<String>,
    ) -> Result<Self, CoreError> {
        if let Some(ref er) = external_reference {
            validate_source_string("external reference", er)?;
        }
        Ok(Self {
            id,
            package,
            event_type,
            source,
            observed_at,
            external_reference,
        })
    }
}

fn validate_source_string(field: &str, s: &str) -> Result<(), CoreError> {
    let kind = match field {
        "external reference" => CoreErrorKind::InvalidName,
        _ => CoreErrorKind::InvalidEventSource,
    };
    if s.is_empty() {
        return Err(CoreError::new(kind, format!("{field} must not be empty")));
    }
    let max = if field == "external reference" {
        EXTERNAL_REFERENCE_MAX
    } else {
        EVENT_SOURCE_MAX
    };
    if s.len() > max {
        return Err(CoreError::new(
            kind,
            format!("{field} length {} exceeds max {max}", s.len()),
        ));
    }
    if s.as_bytes().iter().any(|b| b.is_ascii_control()) {
        return Err(CoreError::new(
            kind,
            format!("{field} contains control characters"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{DistributionFamily, DistributionRef, DistributionRelease};
    use crate::package::PackageName;
    use time::macros::datetime;

    fn pkg() -> PackageIdentity {
        PackageIdentity::new(
            DistributionRef::new(
                DistributionFamily::new("debian").unwrap(),
                DistributionRelease::new("unstable").unwrap(),
            ),
            PackageName::new("foo").unwrap(),
        )
    }

    #[test]
    fn event_source_accepts_canonical_examples() {
        for s in ["upstream-watch", "distro-bts-poll", "manual-trigger"] {
            assert!(EventSource::new(s).is_ok(), "{s} should be valid");
        }
    }

    #[test]
    fn event_source_rejects_empty() {
        assert_eq!(
            EventSource::new("").unwrap_err().kind,
            CoreErrorKind::InvalidEventSource,
        );
    }

    #[test]
    fn event_source_rejects_control_chars() {
        assert_eq!(
            EventSource::new("foo\nbar").unwrap_err().kind,
            CoreErrorKind::InvalidEventSource,
        );
    }

    #[test]
    fn event_type_serializes_as_snake_case() {
        let json = serde_json::to_string(&MaintenanceEventType::SecurityAdvisory).unwrap();
        assert_eq!(json, "\"security_advisory\"");
        let json = serde_json::to_string(&MaintenanceEventType::DependencyTransition).unwrap();
        assert_eq!(json, "\"dependency_transition\"");
    }

    #[test]
    fn event_type_other_carries_string() {
        let json =
            serde_json::to_string(&MaintenanceEventType::Other("custom".to_string())).unwrap();
        let parsed: MaintenanceEventType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, MaintenanceEventType::Other("custom".to_string()));
    }

    #[test]
    fn event_round_trips_with_rfc3339_timestamp() {
        let event = MaintenanceEvent::new(
            MaintenanceEventId::new(),
            pkg(),
            MaintenanceEventType::UpstreamRelease,
            EventSource::new("upstream-watch").unwrap(),
            datetime!(2026-01-15 12:00:00 UTC),
            Some("debian-bts/1234567".to_string()),
        )
        .unwrap();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: MaintenanceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn event_omits_external_reference_when_none() {
        let event = MaintenanceEvent::new(
            MaintenanceEventId::new(),
            pkg(),
            MaintenanceEventType::ManualRequest,
            EventSource::new("manual-trigger").unwrap(),
            datetime!(2026-01-15 12:00:00 UTC),
            None,
        )
        .unwrap();
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("external_reference"));
    }

    #[test]
    fn event_rejects_empty_external_reference() {
        let err = MaintenanceEvent::new(
            MaintenanceEventId::new(),
            pkg(),
            MaintenanceEventType::UpstreamRelease,
            EventSource::new("upstream-watch").unwrap(),
            datetime!(2026-01-15 12:00:00 UTC),
            Some(String::new()),
        )
        .unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }
}
