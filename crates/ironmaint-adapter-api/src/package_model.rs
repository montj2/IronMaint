//! Package model + change classification (§47).
//!
//! [`PackageModelCapability`] describes how the adapter validates
//! package names and classifies repository-relative path changes
//! into the generic [`ChangeDomain`] taxonomy.

use ironmaint_core::PackageName;
use ironmaint_evidence::ChangeDomain;

use crate::error::AdapterError;

/// Classification hint for a [`ChangedPath`].
///
/// The classification is the adapter's *pre-classification* — a
/// hint based on the path and the adapter's package-model
/// conventions. The authoritative classification into
/// [`ChangeDomain`] happens in [`PackageModelCapability::classify_changes`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PathRole {
    UpstreamSource,
    UpstreamTests,
    PackagingMetadata,
    PackagingBuildConfig,
    PackagingRuntimeDeps,
    PackagingTests,
    PackagingPatch,
    LicensingMetadata,
    DocumentationOnly,
    ReleaseMetadata,
    /// Distribution-specific role.
    VendorSpecific(String),
}

/// A repository-relative path the agent classified against a
/// distribution's package model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChangedPath {
    pub path: String,
    pub role: PathRole,
}

impl ChangedPath {
    #[must_use]
    pub fn new(path: impl Into<String>, role: PathRole) -> Self {
        Self {
            path: path.into(),
            role,
        }
    }
}

/// Mandatory capability: validate package names and classify changes.
pub trait PackageModelCapability: Send + Sync + 'static {
    /// Validate `name` against this distribution's grammar.
    ///
    /// # Errors
    /// Returns `AdapterError { kind: InvalidPackage, .. }` if
    /// `name` cannot be expressed in this distribution's syntax.
    fn validate_name(&self, name: &PackageName) -> Result<(), AdapterError>;

    /// Classify `changes` into the set of [`ChangeDomain`]s they
    /// collectively touch.
    ///
    /// The output is the deduplicated set of domains touched;
    /// individual path→domain mapping is the adapter's concern.
    ///
    /// # Errors
    /// Returns `AdapterError` if classification cannot proceed (e.g.
    /// a path hint is unrecognized).
    fn classify_changes(&self, changes: &[ChangedPath]) -> Result<Vec<ChangeDomain>, AdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_path_construction() {
        let p = ChangedPath::new("debian/control", PathRole::PackagingMetadata);
        assert_eq!(p.path, "debian/control");
        assert_eq!(p.role, PathRole::PackagingMetadata);
    }

    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn PackageModelCapability>();
    }
}
