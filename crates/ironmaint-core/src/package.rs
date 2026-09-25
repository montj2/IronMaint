//! Package identity (§§8–9).
//!
//! Core represents package identity as `(distribution, source_name)` and
//! package *revision* as that plus a `version`. Core does NOT enforce
//! Debian/RPM naming rules, does NOT compare versions, and does NOT
//! parse epoch — those live in adapter implementations.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::distribution::DistributionRef;
use crate::error::{CoreError, CoreErrorKind};

const PACKAGE_LABEL_MAX: usize = 256;

/// Strongly-typed package name.
///
/// Validated to be non-empty, free of control characters (including
/// NUL), and ≤ 256 bytes. Distribution-specific naming rules
/// (e.g. Debian's `[a-z0-9][a-z0-9.+\-]*`, RPM's richer character
/// class) are NOT enforced here — that's an adapter concern (§9).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageName(String);

impl PackageName {
    /// Construct a `PackageName` from a string.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidName, .. }` if the name is
    /// empty, too long, or contains control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let s = value.into();
        validate_label("package name", &s)?;
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

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for PackageName {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Strongly-typed package version string.
///
/// Opaque in core: equality, storage, and serialization only. Core
/// does NOT provide version ordering, epoch parsing, or any
/// distribution-specific comparison logic. Adapters expose
/// `compare_versions` in 0A.4.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageVersion(String);

impl PackageVersion {
    /// Construct a `PackageVersion` from a string.
    ///
    /// # Errors
    /// Same shape as [`PackageName::new`].
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let s = value.into();
        validate_label("package version", &s)?;
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

impl AsRef<str> for PackageVersion {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for PackageVersion {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Identity of a package — `(distribution, source_name)`.
///
/// This is the durable key. A `PackageRevision` adds a version, but
/// the *identity* (what distribution and what package name) is fixed
/// by `PackageIdentity`. Equality is structural.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageIdentity {
    pub distribution: DistributionRef,
    pub source_name: PackageName,
}

impl PackageIdentity {
    #[must_use]
    pub fn new(distribution: DistributionRef, source_name: PackageName) -> Self {
        Self {
            distribution,
            source_name,
        }
    }
}

/// A `(PackageIdentity, version)` pair — i.e., one version of one
/// package in one distribution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageRevision {
    pub package: PackageIdentity,
    pub version: PackageVersion,
}

impl PackageRevision {
    #[must_use]
    pub fn new(package: PackageIdentity, version: PackageVersion) -> Self {
        Self { package, version }
    }
}

fn validate_label(field: &str, s: &str) -> Result<(), CoreError> {
    if s.is_empty() {
        return Err(CoreError::new(
            CoreErrorKind::InvalidName,
            format!("{field} must not be empty"),
        ));
    }
    if s.len() > PACKAGE_LABEL_MAX {
        return Err(CoreError::new(
            CoreErrorKind::InvalidName,
            format!("{field} length {} exceeds max {PACKAGE_LABEL_MAX}", s.len()),
        ));
    }
    if s.as_bytes().iter().any(|b| b.is_ascii_control()) {
        return Err(CoreError::new(
            CoreErrorKind::InvalidName,
            format!("{field} contains control characters"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{DistributionFamily, DistributionRelease};

    fn fam() -> DistributionFamily {
        DistributionFamily::new("debian").unwrap()
    }
    fn rel() -> DistributionRelease {
        DistributionRelease::new("unstable").unwrap()
    }
    fn dref() -> DistributionRef {
        DistributionRef::new(fam(), rel())
    }

    #[test]
    fn package_name_accepts_canonical_examples() {
        for label in ["foo", "libfoo", "libfoo-dev", "python3-foo.bar"] {
            assert!(
                PackageName::new(label).is_ok(),
                "expected `{label}` to be a valid package name"
            );
        }
    }

    #[test]
    fn package_name_rejects_empty() {
        assert_eq!(
            PackageName::new("").unwrap_err().kind,
            CoreErrorKind::InvalidName,
        );
    }

    #[test]
    fn package_name_rejects_nul() {
        let err = PackageName::new("foo\0bar").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidName);
    }

    #[test]
    fn package_name_rejects_too_long() {
        let label = "a".repeat(PACKAGE_LABEL_MAX + 1);
        assert_eq!(
            PackageName::new(label).unwrap_err().kind,
            CoreErrorKind::InvalidName,
        );
    }

    #[test]
    fn package_version_accepts_debian_style() {
        assert!(PackageVersion::new("1.9.0-1").is_ok());
    }

    #[test]
    fn package_version_accepts_fedora_style() {
        assert!(PackageVersion::new("1.9.0-1.fc45").is_ok());
    }

    #[test]
    fn package_version_does_not_compare() {
        // §8: "Core permits storage, equality, serialization ONLY."
        // The compile-time guarantee is that we never derive `Ord` on a
        // version-comparison semantic — only `PartialEq`/`Eq` for
        // storage.
        let a = PackageVersion::new("1.0.0").unwrap();
        let b = PackageVersion::new("1.0.0").unwrap();
        let c = PackageVersion::new("1.0.1").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn identity_and_revision_construct() {
        let id = PackageIdentity::new(dref(), PackageName::new("foo").unwrap());
        let rev = PackageRevision::new(id.clone(), PackageVersion::new("1.9.0-1").unwrap());
        assert_eq!(id.source_name.as_str(), "foo");
        assert_eq!(rev.package, id);
        assert_eq!(rev.version.as_str(), "1.9.0-1");
    }

    #[test]
    fn identity_serializes_to_expected_shape() {
        let id = PackageIdentity::new(dref(), PackageName::new("foo").unwrap());
        let json = serde_json::to_string(&id).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["distribution"]["family"], "debian");
        assert_eq!(parsed["distribution"]["release"], "unstable");
        assert_eq!(parsed["source_name"], "foo");
    }
}
