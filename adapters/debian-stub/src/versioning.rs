//! Debian versioning capability (§46, §66, §15).
//!
//! 1B.2: real `dpkg --compare-versions` semantics. The `validate`
//! and `compare` methods delegate to the `debversion` crate
//! (jelmer/debversion-rs, crates.io `debversion`), which
//! implements the dpkg §5.6.12 algorithm directly. The crate
//! accepts the full grammar: `epoch:upstream-debian_revision`
//! with `~` (pre-release marker), `+` (separates upstream from
//! debian_revision and is a binNMU-style marker), `:` (epoch),
//! binNMU `+bN` in debian_revision, and the numeric-runs-as-
//! integers lex-order rule.
//!
//! Per §15, "Do not reimplement Debian version ordering casually."
//! The crate is the named candidate and is actively maintained
//! (last published 2026-06-16, Apache-2.0). The acceptance corpus
//! in `tests/oracle_compare_versions.rs` compares the comparator
//! against `dpkg --compare-versions` when `dpkg` is on `$PATH`
//! (true inside the §97 gate's `ironmaint/workspace:0.1` image).

use std::cmp::Ordering;
use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterError, AdapterErrorKind, VersioningCapability};
use ironmaint_core::PackageVersion;

/// Cached Debian descriptor's family name — only used in error messages
/// to identify which adapter produced the failure.
const FAMILY: &str = "debian";

static DEBIAN_VERSIONING: LazyLock<DebianVersioning> = LazyLock::new(DebianVersioning::new);

/// Debian stub version comparator.
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
        // `debversion::Version`'s `FromStr` impl is the dpkg §5.6.12
        // parser. It accepts the full grammar (epoch, upstream,
        // debian_revision, ~, +, :, binNMU `+bN`) and rejects
        // malformed inputs.
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
        // 1B.2 GREEN: real dpkg --compare-versions semantics. The
        // `debversion` crate implements the §5.6.12 algorithm
        // (numeric-runs-as-integers, `~` before non-`~`, end-of-
        // string before any non-digit, binNMU `+bN` ordering,
        // epoch as numeric prefix). `Version` implements `Ord`
        // and `Eq` to match dpkg's transitive equality (1.0 ==
        // 1.0-0, etc.).
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

/// Return the shared singleton — used by the orchestrator so every
/// call returns the same `&dyn VersioningCapability` reference.
#[must_use]
pub fn debian_versioning() -> &'static DebianVersioning {
    &DEBIAN_VERSIONING
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::PackageVersion;

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
        // 2:1.0 has higher epoch than 1:99.0 — must be Greater regardless
        // of upstream comparison.
        assert_eq!(
            v.compare(&pv("2:1.0"), &pv("1:99.0")).unwrap(),
            Ordering::Greater
        );
    }

    #[test]
    fn tilde_accepted_and_sorts_before_release() {
        // 1B.2: tilde is the dpkg pre-release marker. It validates,
        // and `1.0.0~rc1 < 1.0.0` (the `~` sorts before any non-`~`
        // character and before end-of-string).
        let v = DebianVersioning::new();
        assert!(v.validate(&pv("1.0.0~rc1")).is_ok());
        assert_eq!(
            v.compare(&pv("1.0.0~rc1"), &pv("1.0.0")).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn bin_nmu_sorts_above_base() {
        // 1B.2: binNMU `+bN` in debian_revision sorts after the
        // un-bumped revision. `2.1-1 < 2.1-1+b1 < 2.1-1+b2`.
        let v = DebianVersioning::new();
        assert_eq!(
            v.compare(&pv("2.1-1"), &pv("2.1-1+b1")).unwrap(),
            Ordering::Less
        );
        assert_eq!(
            v.compare(&pv("2.1-1+b1"), &pv("2.1-1+b2")).unwrap(),
            Ordering::Less
        );
        assert_eq!(
            v.compare(&pv("2.1-1"), &pv("2.1-1+b2")).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn numeric_runs_compare_as_integers() {
        // 1B.2: dpkg lex-order compares numeric runs as integers;
        // bytewise String::cmp would say 1.0.0-10 < 1.0.0-2.
        let v = DebianVersioning::new();
        assert_eq!(
            v.compare(&pv("1.0.0-2"), &pv("1.0.0-10")).unwrap(),
            Ordering::Less
        );
        assert_eq!(
            v.compare(&pv("1.0.0-2"), &pv("1.0.0-2")).unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn cross_format_splits_debian_revision() {
        // §46: the stub must produce visibly different comparison
        // results for cross-formatted versions. We document the
        // Debian split here; the cross-stub assertion (Debian vs
        // Fedora) lives in the §46 acceptance integration test.
        let v = DebianVersioning::new();
        let cmp = v.compare(&pv("1.9.0-1.fc45"), &pv("1.9.0-1")).unwrap();
        // Debian split: upstream "1.9.0" = "1.9.0", then
        // debian_revision "1.fc45" > "1" per dpkg lex-order
        // (non-digit char sorts after end of string).
        assert_eq!(cmp, Ordering::Greater);
    }

    #[test]
    fn singleton_returns_same_instance() {
        let a: &dyn VersioningCapability = debian_versioning();
        let b: &dyn VersioningCapability = debian_versioning();
        assert!(std::ptr::eq(a as *const _, b as *const _));
    }
}
