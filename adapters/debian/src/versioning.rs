//! Debian versioning capability (PHASE-1 §15, vendored from
//! `adapters/debian-stub/src/versioning.rs`).
//!
//! The `validate` and `compare` methods delegate to the
//! `debversion` crate, which implements the dpkg §5.6.12
//! algorithm directly. The `~` pre-release marker, binNMU `+bN`
//! in debian_revision, numeric-runs-as-integers lex order, and
//! "missing debian_revision defaults to 0" rules are all handled
//! by the crate; the comparator and validator here are the same
//! shape as the stub's.

use std::cmp::Ordering;
use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterError, AdapterErrorKind, VersioningCapability};
use ironmaint_core::PackageVersion;

/// Cached Debian descriptor's family name — only used in error messages
/// to identify which adapter produced the failure.
const FAMILY: &str = "debian";

static DEBIAN_VERSIONING: LazyLock<DebianVersioning> = LazyLock::new(DebianVersioning::new);

/// Debian version comparator.
pub struct DebianVersioning;

impl DebianVersioning {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianVersioning {
    fn default() -> Self {
        Self::new()
    }
}

impl VersioningCapability for DebianVersioning {
    fn validate(&self, version: &PackageVersion) -> Result<(), AdapterError> {
        let raw = version.as_str();
        raw.parse::<debversion::Version>()
            .map_err(|parse_err| AdapterError {
                kind: AdapterErrorKind::InvalidVersion,
                message: format!("{FAMILY}: invalid version string {raw:?}: {parse_err}"),
            })
            .map(|_| ())
    }

    fn compare(
        &self,
        left: &PackageVersion,
        right: &PackageVersion,
    ) -> Result<Ordering, AdapterError> {
        let l = left
            .as_str()
            .parse::<debversion::Version>()
            .map_err(|parse_err| AdapterError {
                kind: AdapterErrorKind::InvalidVersion,
                message: format!(
                    "{FAMILY}: invalid version string {:?}: {}",
                    left.as_str(),
                    parse_err
                ),
            })?;
        let r = right
            .as_str()
            .parse::<debversion::Version>()
            .map_err(|parse_err| AdapterError {
                kind: AdapterErrorKind::InvalidVersion,
                message: format!(
                    "{FAMILY}: invalid version string {:?}: {}",
                    right.as_str(),
                    parse_err
                ),
            })?;
        Ok(l.cmp(&r))
    }
}

/// Return the shared singleton.
#[must_use]
pub fn debian_versioning() -> &'static DebianVersioning {
    &DEBIAN_VERSIONING
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pv(s: &str) -> PackageVersion {
        PackageVersion::new(s).expect("static literal must be a valid version")
    }

    #[test]
    fn equal_versions_compare_equal() {
        let v = DebianVersioning::new();
        assert_eq!(
            v.compare(&pv("1.0.0-1"), &pv("1.0.0-1")).unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn debian_revision_takes_precedence_after_upstream_equal() {
        let v = DebianVersioning::new();
        assert_eq!(
            v.compare(&pv("1.0.0-1"), &pv("1.0.0-2")).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn epoch_overrides_upstream() {
        let v = DebianVersioning::new();
        assert_eq!(
            v.compare(&pv("2:1.0"), &pv("1:99.0")).unwrap(),
            Ordering::Greater
        );
    }

    #[test]
    fn tilde_accepted_and_sorts_before_release() {
        let v = DebianVersioning::new();
        assert!(v.validate(&pv("1.0.0~rc1")).is_ok());
        assert_eq!(
            v.compare(&pv("1.0.0~rc1"), &pv("1.0.0")).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn singleton_returns_same_instance() {
        let a: &dyn VersioningCapability = debian_versioning();
        let b: &dyn VersioningCapability = debian_versioning();
        assert!(std::ptr::eq(a as *const _, b as *const _));
    }
}
