//! Generic external-issue observations (§37).
//!
//! Issue *snapshots* are pure value objects — they describe what a
//! bug tracker reports about an issue at a moment in time, without
//! any authorization, gate, or obligation coupling. Issue *actions*
//! (which carry [`crate::AuthorizationState`] and reference evidence)
//! live in `ironmaint-policy::issue_action`.
//!
//! Core never interprets the contents of `severity`, `labels`, or
//! `title`; those are opaque to the domain layer and to the state
//! machine. Adapters are free to downgrade any field to `None` when
//! the underlying tracker has no equivalent.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::{CoreError, CoreErrorKind};
use crate::{DistributionRef, IssueProviderId, PackageIdentity};

const EXTERNAL_ID_MAX: usize = 128;

/// Opaque external issue identifier (§37).
///
/// Two halves: which issue tracker it came from ([`IssueProviderId`]),
/// and that tracker's own identifier for the issue. Providers whose
/// ID syntax requires a project namespace can fold it into
/// `external_id` (e.g. `"owner/repo#123"`).
///
/// `external_id` is validated: `1..=128` chars, no control
/// characters. The pattern is intentionally permissive — bug tracker
/// identifiers vary wildly and the spec leaves them opaque.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct IssueRef {
    pub provider: IssueProviderId,
    pub external_id: String,
}

impl IssueRef {
    /// Construct an [`IssueRef`].
    ///
    /// # Errors
    /// Returns `Err(CoreError)` if `external_id` is empty, longer than
    /// 128 bytes, or contains a control character.
    pub fn new(
        provider: IssueProviderId,
        external_id: impl Into<String>,
    ) -> Result<Self, CoreError> {
        let id = external_id.into();
        if id.is_empty() {
            return Err(CoreError::new(
                CoreErrorKind::InvalidName,
                "issue external_id must not be empty",
            ));
        }
        if id.len() > EXTERNAL_ID_MAX {
            return Err(CoreError::new(
                CoreErrorKind::InvalidName,
                format!(
                    "issue external_id length {} exceeds max {EXTERNAL_ID_MAX}",
                    id.len()
                ),
            ));
        }
        if id.as_bytes().iter().any(|b| b.is_ascii_control()) {
            return Err(CoreError::new(
                CoreErrorKind::InvalidName,
                "issue external_id contains control characters",
            ));
        }
        Ok(Self {
            provider,
            external_id: id,
        })
    }
}

impl fmt::Display for IssueRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.external_id)
    }
}

/// Generic issue state vocabulary (§37).
///
/// Covers the common lifecycle seen across BTS, Bugzilla, and trackers
/// of record. The `Other(String)` arm absorbs provider-specific states
/// (e.g. Bodhi `stable`, Koji `failed`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IssueState {
    Open,
    Pending,
    Forwarded,
    Closed,
    /// Provider-specific state the core vocabulary does not enumerate.
    Other(String),
}

impl IssueState {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Open => "open",
            Self::Pending => "pending",
            Self::Forwarded => "forwarded",
            Self::Closed => "closed",
            Self::Other(s) => s,
        }
    }
}

impl fmt::Display for IssueState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A point-in-time snapshot of an external issue (§37).
///
/// `distribution`, `package`, `severity`, and `labels` are all
/// optional — adapters downgrade fields the provider has no
/// equivalent for. Core does not enforce any invariant linking
/// `distribution` to `package.distribution`; that's the adapter's
/// problem.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct IssueSnapshot {
    pub issue: IssueRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<DistributionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageIdentity>,
    pub title: String,
    pub state: IssueState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "crate::json_schema_impls::Rfc3339DateTime")]
    pub observed_at: OffsetDateTime,
}

impl IssueSnapshot {
    #[must_use]
    pub fn new(
        issue: IssueRef,
        title: impl Into<String>,
        state: IssueState,
        observed_at: OffsetDateTime,
    ) -> Self {
        Self {
            issue,
            distribution: None,
            package: None,
            title: title.into(),
            state,
            severity: None,
            labels: Vec::new(),
            observed_at,
        }
    }

    #[must_use]
    pub fn with_distribution(mut self, distribution: DistributionRef) -> Self {
        self.distribution = Some(distribution);
        self
    }

    #[must_use]
    pub fn with_package(mut self, package: PackageIdentity) -> Self {
        self.package = Some(package);
        self
    }

    #[must_use]
    pub fn with_severity(mut self, severity: impl Into<String>) -> Self {
        self.severity = Some(severity.into());
        self
    }

    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.labels.push(label.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DistributionFamily, DistributionRelease};
    use time::macros::datetime;

    fn provider() -> IssueProviderId {
        IssueProviderId::new()
    }

    #[test]
    fn issue_ref_accepts_canonical_examples() {
        let p = provider();
        for id in ["823451", "owner/repo#123", "fedora-1234"] {
            assert!(
                IssueRef::new(p, id).is_ok(),
                "expected `{id}` to be a valid external_id"
            );
        }
    }

    #[test]
    fn issue_ref_rejects_empty() {
        let err = IssueRef::new(provider(), "").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn issue_ref_rejects_oversize() {
        let long = "x".repeat(EXTERNAL_ID_MAX + 1);
        let err = IssueRef::new(provider(), long).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn issue_ref_rejects_control_characters() {
        let err = IssueRef::new(provider(), "foo\nbar").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn issue_ref_display() {
        let p = provider();
        let r = IssueRef::new(p, "823451").unwrap();
        // Display is "<uuid>/<external_id>"; we only assert the suffix
        // and the presence of a slash separator.
        let s = r.to_string();
        assert!(s.ends_with("/823451"), "got `{s}`");
    }

    #[test]
    fn issue_state_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&IssueState::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&IssueState::Forwarded).unwrap(),
            r#""forwarded""#
        );
    }

    #[test]
    fn issue_state_other_carries_string() {
        let s = IssueState::Other("stable".to_string());
        let json = serde_json::to_string(&s).unwrap();
        let parsed: IssueState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn snapshot_builder_chain() {
        let p = provider();
        let issue = IssueRef::new(p, "1").unwrap();
        let dref = DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("sid").unwrap(),
        );
        let s = IssueSnapshot::new(
            issue.clone(),
            "test issue",
            IssueState::Open,
            datetime!(2026-01-01 00:00:00 UTC),
        )
        .with_distribution(dref.clone())
        .with_severity("normal")
        .with_label("upstream")
        .with_label("regression");
        assert_eq!(s.title, "test issue");
        assert_eq!(s.state, IssueState::Open);
        assert_eq!(s.distribution, Some(dref));
        assert_eq!(s.severity.as_deref(), Some("normal"));
        assert_eq!(s.labels, vec!["upstream", "regression"]);
    }

    #[test]
    fn snapshot_omits_none_optionals() {
        let p = provider();
        let issue = IssueRef::new(p, "1").unwrap();
        let s = IssueSnapshot::new(
            issue,
            "t",
            IssueState::Open,
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("distribution"));
        assert!(!json.contains("package"));
        assert!(!json.contains("severity"));
        assert!(!json.contains("labels"));
    }

    #[test]
    fn snapshot_round_trips() {
        let p = provider();
        let issue = IssueRef::new(p, "1").unwrap();
        let s = IssueSnapshot::new(
            issue,
            "title",
            IssueState::Closed,
            datetime!(2026-02-02 12:00:00 UTC),
        )
        .with_severity("high");
        let json = serde_json::to_string(&s).unwrap();
        let parsed: IssueSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }
}
