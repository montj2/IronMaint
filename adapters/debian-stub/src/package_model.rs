//! Debian package-model capability (§47).
//!
//! Classifies paths inside the source tree by role:
//! `debian/control` → packaging metadata, `debian/rules` → build
//! configuration, `debian/tests/*` → tests, `debian/patches/*` → patch
//! set. The visible Fedora classification is intentionally different
//! (spec file vs `debian/control`, `%files`/`%build` vs `debian/rules`).
//!
//! Validation: package names follow Debian Policy §5.6.4 — lowercase
//! alphanumerics plus `+.-`, must start with lowercase alphanumeric,
//! and must be at least two characters.

use std::sync::LazyLock;

use ironmaint_adapter_api::{
    AdapterError, AdapterErrorKind, ChangedPath, PackageModelCapability, PathRole,
};
use ironmaint_core::PackageName;
use ironmaint_evidence::ChangeDomain;

static DEBIAN_PACKAGE_MODEL: LazyLock<DebianPackageModel> = LazyLock::new(DebianPackageModel::new);

pub struct DebianPackageModel;

impl DebianPackageModel {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianPackageModel {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageModelCapability for DebianPackageModel {
    fn validate_name(&self, name: &PackageName) -> Result<(), AdapterError> {
        let raw = name.as_str();
        if raw.len() < 2 {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidPackage,
                message: "debian: package name must be at least 2 characters".to_string(),
            });
        }
        let bytes = raw.as_bytes();
        if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidPackage,
                message: "debian: package name must start with lowercase alphanumeric".to_string(),
            });
        }
        for b in bytes {
            if !(b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(*b, b'+' | b'.' | b'-')) {
                return Err(AdapterError {
                    kind: AdapterErrorKind::InvalidPackage,
                    message: format!(
                        "debian: package name contains illegal character: {}",
                        *b as char
                    ),
                });
            }
        }
        Ok(())
    }

    fn classify_changes(&self, changes: &[ChangedPath]) -> Result<Vec<ChangeDomain>, AdapterError> {
        Ok(changes.iter().map(classify_one).collect())
    }
}

#[must_use]
pub fn debian_package_model() -> &'static DebianPackageModel {
    &DEBIAN_PACKAGE_MODEL
}

fn classify_one(c: &ChangedPath) -> ChangeDomain {
    match (c.path.as_str(), &c.role) {
        (p, _) if p == "debian/control" || p == "debian/compat" || p == "debian/copyright" => {
            ChangeDomain::PackagingMetadata
        }
        (p, _) if p == "debian/rules" || p.starts_with("debian/rules.d/") => {
            ChangeDomain::BuildConfiguration
        }
        (p, _) if p.starts_with("debian/tests/") => ChangeDomain::Tests,
        (p, _) if p.starts_with("debian/patches/") => ChangeDomain::PatchSet,
        (p, _) if p.starts_with("debian/") => ChangeDomain::PackagingMetadata,
        (p, _) if p.ends_with(".md") || p == "README" || p == "README.rst" => {
            ChangeDomain::DocumentationOnly
        }
        (p, _) if p == "LICENSE" || p.starts_with("LICENSES/") => ChangeDomain::LicensingMetadata,
        (_, PathRole::PackagingMetadata) => ChangeDomain::PackagingMetadata,
        (_, PathRole::PackagingBuildConfig) => ChangeDomain::BuildConfiguration,
        (_, PathRole::PackagingTests) => ChangeDomain::Tests,
        (_, PathRole::PackagingPatch) => ChangeDomain::PatchSet,
        (_, PathRole::LicensingMetadata) => ChangeDomain::LicensingMetadata,
        (_, PathRole::DocumentationOnly) => ChangeDomain::DocumentationOnly,
        (_, PathRole::ReleaseMetadata) => ChangeDomain::ReleaseMetadata,
        (_, PathRole::PackagingRuntimeDeps) => ChangeDomain::RuntimeDependencies,
        (_, PathRole::UpstreamTests) => ChangeDomain::Tests,
        (_, PathRole::UpstreamSource) => ChangeDomain::UpstreamSource,
        (_, PathRole::VendorSpecific(_)) => ChangeDomain::AdapterSpecific(c.path.clone()),
        // `PathRole` is `#[non_exhaustive]`; unknown variants fall
        // through to the conservative adapter-specific bucket so
        // consumers keep working as the enum grows.
        (_, _) => ChangeDomain::AdapterSpecific(c.path.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debian_control_is_packaging_metadata() {
        let role = classify_one(&ChangedPath {
            path: "debian/control".to_string(),
            role: PathRole::PackagingMetadata,
        });
        assert_eq!(role, ChangeDomain::PackagingMetadata);
    }

    #[test]
    fn debian_rules_is_build_configuration() {
        let role = classify_one(&ChangedPath {
            path: "debian/rules".to_string(),
            role: PathRole::PackagingBuildConfig,
        });
        assert_eq!(role, ChangeDomain::BuildConfiguration);
    }

    #[test]
    fn debian_tests_directory_classified_as_tests() {
        let role = classify_one(&ChangedPath {
            path: "debian/tests/control".to_string(),
            role: PathRole::PackagingTests,
        });
        assert_eq!(role, ChangeDomain::Tests);
    }

    #[test]
    fn debian_patches_classified_as_patch_set() {
        let role = classify_one(&ChangedPath {
            path: "debian/patches/fix-foo.patch".to_string(),
            role: PathRole::PackagingPatch,
        });
        assert_eq!(role, ChangeDomain::PatchSet);
    }

    #[test]
    fn upstream_source_classified_as_upstream() {
        let role = classify_one(&ChangedPath {
            path: "src/main.c".to_string(),
            role: PathRole::UpstreamSource,
        });
        assert_eq!(role, ChangeDomain::UpstreamSource);
    }

    #[test]
    fn readme_is_documentation_only() {
        let role = classify_one(&ChangedPath {
            path: "README.md".to_string(),
            role: PathRole::DocumentationOnly,
        });
        assert_eq!(role, ChangeDomain::DocumentationOnly);
    }

    #[test]
    fn name_validates_legal_names() {
        let m = DebianPackageModel::new();
        assert!(m.validate_name(&PackageName::new("foo").unwrap()).is_ok());
        assert!(
            m.validate_name(&PackageName::new("libfoo1").unwrap())
                .is_ok()
        );
        assert!(
            m.validate_name(&PackageName::new("libfoo-dev").unwrap())
                .is_ok()
        );
        assert!(
            m.validate_name(&PackageName::new("lib+meta1").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn name_rejects_uppercase() {
        let m = DebianPackageModel::new();
        let err = m
            .validate_name(&PackageName::new("Foo").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidPackage);
    }

    #[test]
    fn name_rejects_too_short() {
        let m = DebianPackageModel::new();
        let err = m
            .validate_name(&PackageName::new("a").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidPackage);
    }

    #[test]
    fn classify_changes_returns_one_domain_per_input() {
        let m = DebianPackageModel::new();
        let changes = vec![
            ChangedPath {
                path: "debian/control".to_string(),
                role: PathRole::PackagingMetadata,
            },
            ChangedPath {
                path: "src/foo.c".to_string(),
                role: PathRole::UpstreamSource,
            },
            ChangedPath {
                path: "debian/tests/control".to_string(),
                role: PathRole::PackagingTests,
            },
        ];
        let domains = m.classify_changes(&changes).unwrap();
        assert_eq!(domains.len(), 3);
        assert_eq!(domains[0], ChangeDomain::PackagingMetadata);
        assert_eq!(domains[1], ChangeDomain::UpstreamSource);
        assert_eq!(domains[2], ChangeDomain::Tests);
    }
}
