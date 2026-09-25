//! Immutable adapter contexts (§53).
//!
//! Adapters receive explicit immutable contexts rather than global
//! service locators. The four context types cover the inputs each
//! capability method needs. All carry borrowed references — no
//! adapter method receives owned copies of the engine's read-model.

use ironmaint_core::{PackageRevision, SourceCandidate};
use ironmaint_policy::{PolicyBaseline, ReleaseCandidate};

use crate::descriptor::AdapterDescriptor;

/// Input to build / package-model capabilities (§53).
#[derive(Debug, Clone, Copy)]
pub struct CandidateContext<'a> {
    pub package: &'a PackageRevision,
    pub candidate: &'a SourceCandidate,
}

/// Input to [`crate::PolicyCapability::derive_obligation_plan`] (§53).
#[derive(Debug, Clone, Copy)]
pub struct PolicyContext<'a> {
    pub package: &'a PackageRevision,
    pub candidate: &'a SourceCandidate,
    pub requested_baseline: Option<&'a PolicyBaseline>,
}

/// Input to [`crate::ReleaseCapability::release_metadata_plan`].
#[derive(Debug, Clone, Copy)]
pub struct ReleaseContext<'a> {
    pub release_candidate: &'a ReleaseCandidate,
    pub adapter: &'a AdapterDescriptor,
}

/// Input to [`crate::ReleaseCapability::publication_plan`].
#[derive(Debug, Clone, Copy)]
pub struct PublicationContext<'a> {
    pub release_candidate: &'a ReleaseCandidate,
    pub adapter: &'a AdapterDescriptor,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::AdapterCapabilities;
    use ironmaint_core::{
        CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
        GitHashAlgorithm, GitObjectId, JobId, PackageIdentity, PackageName, PackageRevision,
        PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
    };
    use ironmaint_policy::PolicyBaseline;
    use time::macros::datetime;
    use url::Url;

    fn hex(c: char, n: usize) -> String {
        std::iter::repeat_n(c, n).collect()
    }

    fn candidate() -> SourceCandidate {
        let repo = RepositoryRef::new(
            VcsKind::Git,
            Url::parse("https://example.invalid/foo.git").unwrap(),
        )
        .unwrap();
        SourceCandidate::new(
            JobId::new(),
            PackageRevision::new(
                PackageIdentity::new(
                    DistributionRef::new(
                        DistributionFamily::new("debian").unwrap(),
                        DistributionRelease::new("sid").unwrap(),
                    ),
                    PackageName::new("foo").unwrap(),
                ),
                PackageVersion::new("1.0.0").unwrap(),
            ),
            repo,
            GitObjectId::new(GitHashAlgorithm::Sha1, hex('a', 40)).unwrap(),
            GitObjectId::new(GitHashAlgorithm::Sha1, hex('b', 40)).unwrap(),
            datetime!(2026-01-01 00:00:00 UTC),
        )
    }

    fn release_candidate() -> ReleaseCandidate {
        ReleaseCandidate::new(
            JobId::new(),
            CandidateFingerprint::from_hex(hex('c', 64)).unwrap(),
            PolicyBaseline::new(DistributionRef::new(
                DistributionFamily::new("debian").unwrap(),
                DistributionRelease::new("sid").unwrap(),
            )),
            datetime!(2026-01-01 00:00:00 UTC),
        )
    }

    #[test]
    fn candidate_context_borrows() {
        let c = candidate();
        let ctx = CandidateContext {
            package: c.package(),
            candidate: &c,
        };
        assert_eq!(ctx.candidate.id(), c.id());
    }

    #[test]
    fn policy_context_with_optional_baseline() {
        let c = candidate();
        let baseline = PolicyBaseline::new(c.package().package.distribution.clone());
        let ctx = PolicyContext {
            package: c.package(),
            candidate: &c,
            requested_baseline: Some(&baseline),
        };
        assert!(ctx.requested_baseline.is_some());
    }

    #[test]
    fn policy_context_without_baseline() {
        let c = candidate();
        let ctx = PolicyContext {
            package: c.package(),
            candidate: &c,
            requested_baseline: None,
        };
        assert!(ctx.requested_baseline.is_none());
    }

    #[test]
    fn release_context_carries_adapter_descriptor() {
        let rc = release_candidate();
        let d = AdapterDescriptor {
            family: DistributionFamily::new("debian").unwrap(),
            implementation_name: "x".into(),
            implementation_version: "0.1.0".into(),
            capabilities: AdapterCapabilities::new(),
        };
        let ctx = ReleaseContext {
            release_candidate: &rc,
            adapter: &d,
        };
        assert_eq!(ctx.release_candidate.id, rc.id);
    }
}
