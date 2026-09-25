//! Debian versioning capability (§46, §66).
//!
//! Phase 0A stub: implements a simplified `epoch:upstream-debian_revision`
//! split that is enough to exercise the §46 acceptance test against
//! the Fedora stub (which uses `epoch:version-release`). Real
//! `dpkg --compare-versions` semantics are explicitly out of scope
//! for 0A (§89).

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
        if raw.contains('~') {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidVersion,
                message: format!("{FAMILY}: tilde (~) not supported by stub comparator"),
            });
        }
        if raw.is_empty() {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidVersion,
                message: format!("{FAMILY}: empty version"),
            });
        }
        Ok(())
    }

    fn compare(
        &self,
        left: &PackageVersion,
        right: &PackageVersion,
    ) -> Result<Ordering, AdapterError> {
        self.validate(left)?;
        self.validate(right)?;
        let lp = parse(left.as_str());
        let rp = parse(right.as_str());
        // Epoch comparison (numeric); absent epoch = 0.
        match lp.epoch.cmp(&rp.epoch) {
            Ordering::Equal => {}
            other => return Ok(other),
        }
        // Upstream / debian_revision comparison (bytewise; stub does
        // not implement dpkg's special lex-order rules — the visible
        // difference vs Fedora's comparator is what §46 demands).
        match lp.upstream.cmp(&rp.upstream) {
            Ordering::Equal => {}
            other => return Ok(other),
        }
        Ok(lp.debian_revision.cmp(&rp.debian_revision))
    }
}

/// Return the shared singleton — used by the orchestrator so every
/// call returns the same `&dyn VersioningCapability` reference.
#[must_use]
pub fn debian_versioning() -> &'static DebianVersioning {
    &DEBIAN_VERSIONING
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SplitVersion {
    epoch: u32,
    upstream: String,
    debian_revision: String,
}

fn parse(raw: &str) -> SplitVersion {
    // epoch may be present as a leading "N:" marker.
    let (epoch, after_epoch) = match raw.find(':') {
        Some(idx) => {
            let head = &raw[..idx];
            let epoch = head.parse::<u32>().unwrap_or(0);
            (epoch, &raw[idx + 1..])
        }
        None => (0u32, raw),
    };

    // The "debian_revision" is whatever appears after the last "-".
    let (upstream, debian_revision) = match after_epoch.rfind('-') {
        Some(idx) => (&after_epoch[..idx], after_epoch[idx + 1..].to_string()),
        None => (after_epoch, String::new()),
    };
    SplitVersion {
        epoch,
        upstream: upstream.to_string(),
        debian_revision,
    }
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
    fn tilde_versions_rejected() {
        let v = DebianVersioning::new();
        let err = v.validate(&pv("1.0.0~rc1")).unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidVersion);
        assert!(err.message.contains('~'));
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
        // debian_revision "1.fc45" > "1" lexicographically.
        assert_eq!(cmp, Ordering::Greater);
    }

    #[test]
    fn singleton_returns_same_instance() {
        let a: &dyn VersioningCapability = debian_versioning();
        let b: &dyn VersioningCapability = debian_versioning();
        assert!(std::ptr::eq(a as *const _, b as *const _));
    }
}
