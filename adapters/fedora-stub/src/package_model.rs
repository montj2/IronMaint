//! Fedora package-model capability (§47).
//!
//! Classifies paths inside the source tree by role:
//! `*.spec` → packaging metadata, `sources` → packaging metadata,
//! `%files`/`%build`/`%install` section markers → build configuration.
//! Distinct from Debian's `debian/control` / `debian/rules` mapping
//! (§80 visible-difference test).

use std::sync::LazyLock;

use ironmaint_adapter_api::{
    AdapterError, AdapterErrorKind, ChangedPath, PackageModelCapability, PathRole,
};
use ironmaint_core::PackageName;
use ironmaint_evidence::ChangeDomain;

static FEDORA_PACKAGE_MODEL: LazyLock<FedoraPackageModel> = LazyLock::new(FedoraPackageModel::new);

pub struct FedoraPackageModel;

impl FedoraPackageModel {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FedoraPackageModel {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageModelCapability for FedoraPackageModel {
    fn validate_name(&self, name: &PackageName) -> Result<(), AdapterError> {
        let raw = name.as_str();
        if raw.len() < 2 {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidPackage,
                message: "fedora: package name must be at least 2 characters".to_string(),
            });
        }
        let bytes = raw.as_bytes();
        // Fedora RPM package names: lowercase alphanumeric plus '.', '-', '+', '_'.
        // Must start with alphanumeric.
        if !bytes[0].is_ascii_alphanumeric() {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidPackage,
                message: "fedora: package name must start with alphanumeric".to_string(),
            });
        }
        for b in bytes {
            if !(b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(*b, b'+' | b'.' | b'-' | b'_'))
            {
                return Err(AdapterError {
                    kind: AdapterErrorKind::InvalidPackage,
                    message: format!(
                        "fedora: package name contains illegal character: {}",
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
pub fn fedora_package_model() -> &'static FedoraPackageModel {
    &FEDORA_PACKAGE_MODEL
}

fn classify_one(c: &ChangedPath) -> ChangeDomain {
    match (c.path.as_str(), &c.role) {
        (p, _) if p.ends_with(".spec") => ChangeDomain::PackagingMetadata,
        (p, _) if p == "sources" || p == ".gitignore" || p == "Makefile" => {
            ChangeDomain::PackagingMetadata
        }
        (p, _) if p.starts_with("%") => ChangeDomain::BuildConfiguration,
        (p, _) if p.starts_with("tests/") || p.starts_with("test/") => ChangeDomain::Tests,
        (p, _) if p == "README.md" || p.ends_with(".md") || p == "README" => {
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
        (_, _) => ChangeDomain::AdapterSpecific(c.path.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_file_is_packaging_metadata() {
        let role = classify_one(&ChangedPath {
            path: "foo.spec".to_string(),
            role: PathRole::PackagingMetadata,
        });
        assert_eq!(role, ChangeDomain::PackagingMetadata);
    }

    #[test]
    fn sources_file_is_packaging_metadata() {
        let role = classify_one(&ChangedPath {
            path: "sources".to_string(),
            role: PathRole::PackagingMetadata,
        });
        assert_eq!(role, ChangeDomain::PackagingMetadata);
    }

    #[test]
    fn macro_marker_is_build_configuration() {
        let role = classify_one(&ChangedPath {
            path: "%build".to_string(),
            role: PathRole::PackagingBuildConfig,
        });
        assert_eq!(role, ChangeDomain::BuildConfiguration);
    }

    #[test]
    fn tests_directory_classified_as_tests() {
        let role = classify_one(&ChangedPath {
            path: "tests/run.sh".to_string(),
            role: PathRole::PackagingTests,
        });
        assert_eq!(role, ChangeDomain::Tests);
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
    fn name_validates_legal_names() {
        let m = FedoraPackageModel::new();
        assert!(m.validate_name(&PackageName::new("foo").unwrap()).is_ok());
        assert!(
            m.validate_name(&PackageName::new("libfoo1").unwrap())
                .is_ok()
        );
        assert!(
            m.validate_name(&PackageName::new("python3-foo").unwrap())
                .is_ok()
        );
        assert!(
            m.validate_name(&PackageName::new("foo_bar").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn name_rejects_uppercase() {
        let m = FedoraPackageModel::new();
        let err = m
            .validate_name(&PackageName::new("Foo").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidPackage);
    }

    #[test]
    fn name_rejects_too_short() {
        let m = FedoraPackageModel::new();
        let err = m
            .validate_name(&PackageName::new("a").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidPackage);
    }
}
