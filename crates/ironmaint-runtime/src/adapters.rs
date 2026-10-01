//! The distribution adapter registry (PHASE-0B.md §67 "load
//! adapter registry", §44 adapter dispatch).
//!
//! A registry maps a [`DistributionFamily`] to the
//! [`DistributionAdapter`] that describes that distribution. The
//! runtime consults it when a candidate is captured, to derive the
//! checks and obligations that distribution requires (PHASE-0B.md
//! §45 check planning, §48 policy derivation).
//!
//! ## Why a registry and not a single adapter
//!
//! The runtime is distribution-neutral (CLAUDE.md: "No Debian/RPM
//! semantics in core"). Branching on family identity in the runtime
//! is forbidden; a *lookup* keyed by the job's own family is not —
//! the runtime never learns what a family means, only which adapter
//! claims to describe it. That keeps `adapters/debian-stub` and
//! `adapters/fedora-stub` out of the runtime's dependency graph
//! while still letting one process serve both.
//!
//! ## Empty registries are valid
//!
//! A job whose family has no registered adapter is *not* an error.
//! The candidate is still captured; it simply has no derived
//! checks or obligations, and the capture's side-effect list says
//! so. That is the state 0B is in for every family whose real
//! tools do not yet exist, and it is better than a panic or a
//! silent empty write.

use std::collections::BTreeMap;
use std::sync::Arc;

use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_core::DistributionFamily;

/// Family-keyed collection of distribution adapters.
///
/// Cheap to clone: the map holds `Arc<dyn DistributionAdapter>`
/// handles, and adapters are themselves required to be
/// `Send + Sync + 'static` so they can be shared across the
/// concurrent `RuntimeService` handle tree.
#[derive(Clone, Default)]
pub struct AdapterRegistry {
    by_family: BTreeMap<String, Arc<dyn DistributionAdapter>>,
}

/// Hand-written because `dyn DistributionAdapter` is not `Debug`:
/// the trait has no `Debug` bound and adding one would force every
/// adapter to derive it for no gain. The registry's own state is
/// the family→adapter *mapping*, so that is what prints.
impl std::fmt::Debug for AdapterRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterRegistry")
            .field("families", &self.families())
            .finish_non_exhaustive()
    }
}

impl AdapterRegistry {
    /// An empty registry. Every family lookup misses.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            by_family: BTreeMap::new(),
        }
    }

    /// Register `adapter` under the family its own descriptor
    /// claims.
    ///
    /// The key is read from [`DistributionAdapter::descriptor`],
    /// never passed in, so an adapter cannot be filed under a
    /// family it does not implement.
    pub fn register(&mut self, adapter: Arc<dyn DistributionAdapter>) -> &mut Self {
        let family = adapter.descriptor().family;
        self.by_family.insert(family.as_str().to_string(), adapter);
        self
    }

    /// Register a boxed adapter, the shape callers building a
    /// registry from a `Vec<Box<dyn …>>` have.
    pub fn register_boxed(&mut self, adapter: Box<dyn DistributionAdapter>) -> &mut Self {
        self.register(Arc::from(adapter))
    }

    /// Look up the adapter claiming `family`.
    #[must_use]
    pub fn get(&self, family: &DistributionFamily) -> Option<&Arc<dyn DistributionAdapter>> {
        self.by_family.get(family.as_str())
    }

    /// Whether any adapter claims `family`.
    #[must_use]
    pub fn contains(&self, family: &DistributionFamily) -> bool {
        self.by_family.contains_key(family.as_str())
    }

    /// The registered families, sorted. Used for startup logging.
    #[must_use]
    pub fn families(&self) -> Vec<&str> {
        self.by_family.keys().map(String::as_str).collect()
    }

    /// How many adapters are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_family.len()
    }

    /// Whether the registry holds no adapters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_family.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_adapter_api::{
        AdapterCapabilities, AdapterDescriptor, BuildCapability, IssueCapability,
        PackageModelCapability, PolicyCapability, ReleaseCapability, VersioningCapability,
    };
    use ironmaint_core::DistributionFamily;

    struct Stub {
        family: DistributionFamily,
    }

    impl DistributionAdapter for Stub {
        fn descriptor(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                family: self.family.clone(),
                implementation_name: "stub".into(),
                implementation_version: "0.0.0".into(),
                capabilities: AdapterCapabilities::new(),
            }
        }
        fn versioning(&self) -> &dyn VersioningCapability {
            unimplemented!("not exercised by the registry")
        }
        fn package_model(&self) -> &dyn PackageModelCapability {
            unimplemented!("not exercised by the registry")
        }
        fn policy(&self) -> Option<&dyn PolicyCapability> {
            None
        }
        fn build(&self) -> Option<&dyn BuildCapability> {
            None
        }
        fn issues(&self) -> Option<&dyn IssueCapability> {
            None
        }
        fn release(&self) -> Option<&dyn ReleaseCapability> {
            None
        }
    }

    fn family(s: &str) -> DistributionFamily {
        DistributionFamily::new(s).unwrap()
    }

    #[test]
    fn empty_registry_misses_every_family() {
        let reg = AdapterRegistry::empty();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.get(&family("debian")).is_none());
        assert!(reg.families().is_empty());
    }

    #[test]
    fn register_keys_on_the_adapters_own_descriptor_family() {
        let mut reg = AdapterRegistry::empty();
        reg.register(Arc::new(Stub {
            family: family("debian"),
        }));
        assert!(reg.get(&family("debian")).is_some());
        assert!(reg.get(&family("fedora")).is_none());
        assert!(reg.contains(&family("debian")));
        assert_eq!(reg.families(), vec!["debian"]);
    }

    #[test]
    fn a_second_registration_for_a_family_replaces_the_first() {
        let mut reg = AdapterRegistry::empty();
        reg.register(Arc::new(Stub {
            family: family("debian"),
        }));
        reg.register(Arc::new(Stub {
            family: family("debian"),
        }));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn families_are_reported_sorted() {
        let mut reg = AdapterRegistry::empty();
        for f in ["fedora", "debian", "arch"] {
            reg.register(Arc::new(Stub { family: family(f) }));
        }
        assert_eq!(reg.families(), vec!["arch", "debian", "fedora"]);
    }

    #[test]
    fn register_boxed_uses_the_same_keying() {
        let mut reg = AdapterRegistry::empty();
        reg.register_boxed(Box::new(Stub {
            family: family("fedora"),
        }));
        assert!(reg.contains(&family("fedora")));
    }

    #[test]
    fn cloning_shares_the_underlying_map() {
        let mut reg = AdapterRegistry::empty();
        reg.register(Arc::new(Stub {
            family: family("debian"),
        }));
        let cloned = reg.clone();
        assert_eq!(cloned.len(), 1);
        // Mutating the clone does not affect the original.
        let mut mutated = cloned;
        mutated.register(Arc::new(Stub {
            family: family("fedora"),
        }));
        assert_eq!(mutated.len(), 2);
        assert_eq!(reg.len(), 1);
    }
}
