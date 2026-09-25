//! Fedora versioning capability (§46, §67).
//!
//! Phase 0A stub: implements a simplified `epoch:version-release`
//! split with `.fcNN` / `.elNN` stripped from `release`. Real RPM
//! EVR semantics are explicitly out of scope for 0A (§89).
//!
//! Visible difference vs Debian (§46): the cross-format
//! `1.9.0-1.fc45` vs `1.9.0-1` is split differently — Fedora
//! recognises the `.fcNN` tag as part of the *release* field, while
//! Debian treats it as part of the debian-revision field.

use std::cmp::Ordering;
use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterError, AdapterErrorKind, VersioningCapability};
use ironmaint_core::PackageVersion;

const FAMILY: &str = "fedora";

static FEDORA_VERSIONING: LazyLock<FedoraVersioning> = LazyLock::new(FedoraVersioning::new);

pub struct FedoraVersioning;

impl FedoraVersioning {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FedoraVersioning {
    fn default() -> Self {
        Self::new()
    }
}

impl VersioningCapability for FedoraVersioning {
    fn validate(&self, version: &PackageVersion) -> Result<(), AdapterError> {
        let raw = version.as_str();
        if raw.is_empty() {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidVersion,
                message: format!("{FAMILY}: empty version"),
            });
        }
        // Fedora rejects multiple colons (epoch must be a leading
        // marker, not embedded).
        if raw.matches(':').count() > 1 {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidVersion,
                message: format!("{FAMILY}: epoch must be a single leading marker"),
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
        match lp.epoch.cmp(&rp.epoch) {
            Ordering::Equal => {}
            other => return Ok(other),
        }
        match lp.version.cmp(&rp.version) {
            Ordering::Equal => {}
            other => return Ok(other),
        }
        Ok(lp.release.cmp(&rp.release))
    }
}

#[must_use]
pub fn fedora_versioning() -> &'static FedoraVersioning {
    &FEDORA_VERSIONING
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SplitVersion {
    epoch: u32,
    version: String,
    release: String,
}

fn parse(raw: &str) -> SplitVersion {
    let (epoch, after_epoch) = match raw.find(':') {
        Some(idx) => {
            let head = &raw[..idx];
            let epoch = head.parse::<u32>().unwrap_or(0);
            (epoch, &raw[idx + 1..])
        }
        None => (0u32, raw),
    };

    // The "release" is whatever appears after the LAST "-", then
    // stripped of any trailing ".fcNN" / ".elNN" tag.
    let (version_part, release_part) = match after_epoch.rfind('-') {
        Some(idx) => (&after_epoch[..idx], &after_epoch[idx + 1..]),
        None => (after_epoch, ""),
    };
    let release = strip_dist_tag(release_part);

    SplitVersion {
        epoch,
        version: version_part.to_string(),
        release: release.to_string(),
    }
}

/// Strip trailing `.fcNN`, `.elNN`, `.epelNN`, `.module+NN` style
/// distribution tags from a release field. The Fedora stub does
/// not implement a full grammar — this is a 0A simplification.
fn strip_dist_tag(release: &str) -> &str {
    for tag in [".fc", ".el", ".epel", ".module+"] {
        if let Some(idx) = release.find(tag) {
            // The tag starts at idx and consumes to end of string.
            return &release[..idx];
        }
    }
    release
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
        let v = FedoraVersioning::new();
        assert_eq!(
            v.compare(&pv("1.0.0-1"), &pv("1.0.0-1")).unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn release_takes_precedence_after_version_equal() {
        let v = FedoraVersioning::new();
        assert_eq!(
            v.compare(&pv("1.0.0-1"), &pv("1.0.0-2")).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn epoch_overrides_version() {
        let v = FedoraVersioning::new();
        assert_eq!(
            v.compare(&pv("2:1.0"), &pv("1:99.0")).unwrap(),
            Ordering::Greater
        );
    }

    #[test]
    fn multi_colon_rejected() {
        let v = FedoraVersioning::new();
        let err = v.validate(&pv("1:2:3")).unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidVersion);
    }

    #[test]
    fn tilde_versions_accepted_by_fedora() {
        // §46 acceptance: tilde versions are accepted by the
        // Fedora stub and rejected by the Debian stub.
        let v = FedoraVersioning::new();
        assert!(v.validate(&pv("1.0.0~rc1")).is_ok());
    }

    #[test]
    fn fc_tag_stripped_for_release_comparison() {
        // Same version, different fc tag → equal under Fedora split.
        let v = FedoraVersioning::new();
        assert_eq!(
            v.compare(&pv("1.9.0-1.fc45"), &pv("1.9.0-1")).unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn singleton_returns_same_instance() {
        let a: &dyn VersioningCapability = fedora_versioning();
        let b: &dyn VersioningCapability = fedora_versioning();
        assert!(std::ptr::eq(a as *const _, b as *const _));
    }
}
