//! Versioning capability (§46).
//!
//! The versioning capability normalizes version strings (per-distribution)
//! and provides ordering. Phase 0A stubs implement a deterministic
//! comparator demonstrating visibly different behavior; the §46
//! acceptance test lives in
//! `crates/ironmaint-adapter-api/tests/version_comparison_differs.rs`.

use std::cmp::Ordering;

use ironmaint_core::PackageVersion;

use crate::error::AdapterError;

/// Compare / validate package versions for one distribution (§46).
///
/// Mandatory capability — every adapter must implement it.
pub trait VersioningCapability: Send + Sync + 'static {
    /// Validate `version` against this distribution's grammar.
    ///
    /// # Errors
    /// Returns `AdapterError { kind: InvalidVersion, .. }` if
    /// `version` cannot be expressed in this distribution's syntax.
    fn validate(&self, version: &PackageVersion) -> Result<(), AdapterError>;

    /// Compare two versions.
    ///
    /// # Errors
    /// Returns `AdapterError { kind: InvalidVersion, .. }` if either
    /// input is unparseable.
    fn compare(
        &self,
        left: &PackageVersion,
        right: &PackageVersion,
    ) -> Result<Ordering, AdapterError>;
}

#[cfg(test)]
mod tests {
    // The trait has no in-module types to round-trip; the §46 acceptance
    // test lives at the workspace level (see tests/version_comparison_differs.rs).
    // Keep a tiny smoke test here so the module is exercised by `cargo test`.
    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn crate::VersioningCapability>();
    }
}
