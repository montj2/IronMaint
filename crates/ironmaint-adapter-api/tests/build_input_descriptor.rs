//! Conformance integration test for
//! `BuildInputDescriptor` (PHASE-2 PR 2.0-D,
//! `doc/PHASE-2-D.md`).
//!
//! The descriptor is the *what* (which candidate, which
//! source format, which upstream tarball) of the
//! candidate/build boundary. The conformance integration
//! test pins the descriptor shape so future PRs that
//! extend the descriptor (e.g. 2A.2's `SourceFormat`
//! enum) cannot silently change the public field
//! layout.
//!
//! The unit tests inside `build_input.rs` cover the
//! behaviour (filename accessor, error formatting). This
//! file is the **shape** test — it asserts the
//! constructor returns a descriptor with the expected
//! fields and equality semantics.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use ironmaint_adapter_api::{BuildInputDescriptor, UpstreamTarball};
use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate,
    VcsKind,
};
use time::macros::datetime;
use url::Url;

fn hex(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

fn make_candidate() -> SourceCandidate {
    let repo = RepositoryRef::new(
        VcsKind::Git,
        Url::parse("https://example.invalid/hello.git").unwrap(),
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
                PackageName::new("hello").unwrap(),
            ),
            PackageVersion::new("2.10-2").unwrap(),
        ),
        repo,
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('a', 40)).unwrap(),
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('b', 40)).unwrap(),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

fn make_tarball() -> UpstreamTarball {
    UpstreamTarball {
        path: PathBuf::from("/var/lib/ironmaint/fixtures/hello_2.10.orig.tar.gz"),
        sha256: [0xab; 32],
    }
}

#[test]
fn build_input_descriptor_quilt_pins_shape() {
    // The descriptor's public shape (2.0-D): the
    // `quilt(...)` constructor returns a descriptor
    // with `candidate: SourceCandidate` and
    // `source_format: SourceFormatQuilt` whose
    // `upstream_tarball: UpstreamTarball` is the
    // tarball the constructor was passed. Equality
    // is structural (the struct derives `PartialEq`).
    let candidate = make_candidate();
    let tarball = make_tarball();
    let descriptor = BuildInputDescriptor::quilt(candidate.clone(), tarball.clone());

    // The candidate is the same value.
    assert_eq!(descriptor.candidate, candidate);

    // The source format is quilt, and it wraps the
    // tarball verbatim (no transformation).
    assert_eq!(descriptor.source_format.upstream_tarball, tarball);

    // The descriptor equals another descriptor built
    // from the same inputs.
    let descriptor_again = BuildInputDescriptor::quilt(candidate.clone(), tarball.clone());
    assert_eq!(descriptor, descriptor_again);

    // A descriptor built from a different candidate is
    // NOT equal — the candidate is part of the
    // identity.
    let other_candidate = make_candidate();
    assert!(other_candidate != candidate);
    let other_descriptor = BuildInputDescriptor::quilt(other_candidate, tarball.clone());
    assert_ne!(descriptor, other_descriptor);
}

#[test]
fn build_input_descriptor_tarball_filename_is_deterministic() {
    // The tarball's `filename()` accessor returns the
    // path's basename. The conformance integration
    // test pins this so the runtime can rely on it
    // for `dpkg-source -b`'s name-based discovery.
    let tarball = UpstreamTarball {
        path: PathBuf::from("/var/lib/ironmaint/fixtures/hello_2.10.orig.tar.gz"),
        sha256: [0; 32],
    };
    assert_eq!(tarball.filename(), "hello_2.10.orig.tar.gz");
}
