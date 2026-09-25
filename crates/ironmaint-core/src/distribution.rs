//! Distribution identity (§7).
//!
//! Distribution is opaque to core. We store strings; we never interpret
//! them as a Debian-vs-Fedora enum. New distributions are added by
//! instantiating a `DistributionFamily` with a new label — no code
//! change is required (PHASE-0A.md §7: "Do not make distributions a
//! Rust enum. This would require modifying core whenever a
//! distribution is added.").

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreErrorKind};

/// Maximum length of a `DistributionFamily` or `DistributionRelease` label.
const LABEL_MAX: usize = 64;

/// Strongly-typed distribution family (e.g. `debian`, `fedora`).
///
/// Validated to `^[a-z0-9][a-z0-9._-]*$` and ≤ 64 bytes. The regex
/// permits the conventional characters seen across distributions
/// today (`debian`, `fedora`, `ubuntu`, `epel`, `opensuse`, `arch`,
/// `mageia`, …) without baking any of them into the type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DistributionFamily(String);

impl DistributionFamily {
    /// Construct a `DistributionFamily` from a string label.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidName, .. }` if the label is
    /// empty, too long, or contains characters outside `[a-z0-9._-]`.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let s = value.into();
        validate_label("distribution family", &s)?;
        Ok(Self(s))
    }

    /// Borrow the underlying label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the underlying `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for DistributionFamily {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DistributionFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for DistributionFamily {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Strongly-typed distribution release identifier (e.g. `unstable`,
/// `trixie`, `epel10`, `rawhide`).
///
/// Same shape and constraints as [`DistributionFamily`] — kept as a
/// distinct type so adapters can read intent at the call site.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DistributionRelease(String);

impl DistributionRelease {
    /// Construct a `DistributionRelease` from a string label.
    ///
    /// # Errors
    /// Same as [`DistributionFamily::new`].
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let s = value.into();
        validate_label("distribution release", &s)?;
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for DistributionRelease {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DistributionRelease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for DistributionRelease {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Pair of `DistributionFamily` + `DistributionRelease`.
///
/// The core represents the authority-target identity as a `family` +
/// `release` pair. The adapter interprets the pair. Nothing here
/// encodes Debian-vs-Fedora semantics.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DistributionRef {
    pub family: DistributionFamily,
    pub release: DistributionRelease,
}

impl DistributionRef {
    #[must_use]
    pub fn new(family: DistributionFamily, release: DistributionRelease) -> Self {
        Self { family, release }
    }
}

/// Validate a `DistributionFamily` / `DistributionRelease` label.
///
/// Hand-rolled byte-level check — no `regex` dep needed because the
/// allowed shape is small and well-defined.
fn validate_label(field: &str, s: &str) -> Result<(), CoreError> {
    if s.is_empty() {
        return Err(CoreError::new(
            CoreErrorKind::InvalidName,
            format!("{field} must not be empty"),
        ));
    }
    if s.len() > LABEL_MAX {
        return Err(CoreError::new(
            CoreErrorKind::InvalidName,
            format!("{field} length {} exceeds max {LABEL_MAX}", s.len()),
        ));
    }
    let bytes = s.as_bytes();
    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(CoreError::new(
            CoreErrorKind::InvalidName,
            format!("{field} `{s}` must start with [a-z0-9]"),
        ));
    }
    for &b in &bytes[1..] {
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-') {
            return Err(CoreError::new(
                CoreErrorKind::InvalidName,
                format!("{field} `{s}` contains invalid byte {b:#04x}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_accepts_canonical_examples() {
        for label in ["debian", "fedora", "ubuntu", "epel", "opensuse", "arch"] {
            assert!(
                DistributionFamily::new(label).is_ok(),
                "expected `{label}` to be a valid family"
            );
        }
    }

    #[test]
    fn release_accepts_canonical_examples() {
        for label in [
            "unstable", "stable", "rawhide", "epel10", "trixie", "bookworm",
        ] {
            assert!(
                DistributionRelease::new(label).is_ok(),
                "expected `{label}` to be a valid release"
            );
        }
    }

    #[test]
    fn family_rejects_uppercase() {
        let err = DistributionFamily::new("Debian").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn family_rejects_empty() {
        let err = DistributionFamily::new("").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
        assert!(err.message.contains("empty"));
    }

    #[test]
    fn family_rejects_too_long() {
        let label = "a".repeat(LABEL_MAX + 1);
        let err = DistributionFamily::new(label).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn family_rejects_leading_dash() {
        let err = DistributionFamily::new("-debian").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn family_rejects_internal_space() {
        let err = DistributionFamily::new("foo bar").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn family_accepts_max_length() {
        let label = "a".repeat(LABEL_MAX);
        assert!(DistributionFamily::new(label).is_ok());
    }

    #[test]
    fn family_round_trips_through_from_str() {
        let f: DistributionFamily = "debian".parse().unwrap();
        assert_eq!(f.as_str(), "debian");
        assert_eq!(f.to_string(), "debian");
    }

    #[test]
    fn ref_is_constructed_from_valid_components() {
        let r = DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        );
        assert_eq!(r.family.as_str(), "debian");
        assert_eq!(r.release.as_str(), "unstable");
    }

    #[test]
    fn ref_serializes_to_expected_shape() {
        let r = DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        );
        let json = serde_json::to_string(&r).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["family"], "debian");
        assert_eq!(parsed["release"], "unstable");
    }
}
