//! `BuildInputDescriptor` — how a captured candidate tree
//! becomes the input to a real build.
//!
//! PHASE-2 PR 2.0-D (make the candidate/build boundary
//! explicit, `doc/PHASE-2-D.md`). The descriptor is the
//! *what*: which candidate, which source format, which
//! upstream tarball. The runtime turns it into a
//! `BuildInputHandle` (in `ironmaint-workspace`) which is
//! the *where*: the directory the build tool will see.
//!
//! The descriptor is **distribution-typed**: the
//! `SourceFormatQuilt` variant names Debian 3.0 (quilt)
//! packaging, the upstream tarball's filename follows the
//! Debian convention (`<pkg>_<ver>.orig.tar.gz`), and
//! the SHA-256 is the on-disk hash of that tarball. The
//! runtime does NOT need to know the convention — it
//! only needs the descriptor to know.
//!
//! ## Side-table pattern
//!
//! 2A.2's `BuildPlan` will carry a `BuildInputDescriptor`
//! reference, but the descriptor and the plan are
//! decoupled in 2.0-D so the descriptor can be built
//! before the plan, and so the conformance suite can
//! test the descriptor in isolation. The descriptor is
//! not a field on `BuildPlan` today; 2A.2's call.

use std::path::PathBuf;

use thiserror::Error;

use ironmaint_core::SourceCandidate;

/// The input to a build: which candidate, which source
/// format, which upstream tarball.
///
/// `candidate` is the captured `SourceCandidate` whose
/// tree will be staged. The runtime reads
/// `candidate.tree` (the BLAKE3-anchored tree OID) and
/// re-derives it at the staged workdir to verify the
/// staging was lossless. `source_format` is the only
/// supported format for 2.0-D: Debian 3.0 (quilt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInputDescriptor {
    pub candidate: SourceCandidate,
    pub source_format: SourceFormatQuilt,
}

impl BuildInputDescriptor {
    /// Construct a 3.0 (quilt) descriptor.
    ///
    /// The `upstream_tarball.path` is the location of the
    /// tarball on disk (typically inside the daemon's
    /// state dir, but the runtime does not constrain
    /// it). The `sha256` is the expected SHA-256 of the
    /// file at that path; the staging primitive compares
    /// against the on-disk hash and rejects a mismatch
    /// before placing the tarball.
    #[must_use]
    pub fn quilt(candidate: SourceCandidate, upstream_tarball: UpstreamTarball) -> Self {
        Self {
            candidate,
            source_format: SourceFormatQuilt { upstream_tarball },
        }
    }
}

/// The 3.0 (quilt) Debian source format.
///
/// The `upstream_tarball` is the source's
/// `<pkg>_<ver>.orig.tar.gz` (or `.xz`/`.bz2`/`.zst` —
/// the extension is the operator's choice; the runtime
/// does not need to inspect it).
///
/// 2.0-D supports **only** 3.0 (quilt). A future PR
/// (2A.2 or beyond) may add a `SourceFormatNative` /
/// `SourceFormatOnePointZero` variant; today's
/// `BuildInputError::UnsupportedSourceFormat` is
/// reserved for that future PR's call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFormatQuilt {
    pub upstream_tarball: UpstreamTarball,
}

/// The upstream orig tarball for a 3.0 (quilt) build.
///
/// `path` is the location of the tarball on disk.
/// `sha256` is the expected SHA-256 of the file at that
/// path; the staging primitive computes the hash and
/// rejects a mismatch with
/// [`BuildInputError::UpstreamTarballSha256Mismatch`].
///
/// The struct does not carry the upstream source's
/// version or filename — those are the operator's
/// concern. The runtime places the tarball at
/// `<staged>/<basename(path)>`, which the Debian
/// convention requires to match
/// `<pkg>_<ver>.orig.tar.gz`; a fixture that does not
/// follow the convention is a fixture bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamTarball {
    pub path: PathBuf,
    pub sha256: [u8; 32],
}

impl UpstreamTarball {
    /// Filename the staging primitive uses to place the
    /// tarball inside the staged workdir. The runtime
    /// does not transform the basename; whatever the
    /// operator provides is what `dpkg-source -b` will
    /// see.
    #[must_use]
    pub fn filename(&self) -> &str {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("upstream.tar.gz")
    }
}

/// Errors a `BuildInputDescriptor` consumer can raise
/// before staging (i.e. at descriptor-construction or
/// pre-stage-validation time). Runtime errors during
/// staging (git archive failures, tar failures, IO)
/// are `WorkspaceError` from
/// `ironmaint-workspace`; the descriptor-side errors
/// are pre-stage validation only.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BuildInputError {
    /// The on-disk upstream tarball's SHA-256 did not
    /// match the descriptor's `upstream_tarball.sha256`.
    /// Expected and actual are hex (lowercase).
    #[error("upstream tarball SHA-256 mismatch: expected {expected}, got {actual}")]
    UpstreamTarballSha256Mismatch { expected: String, actual: String },

    /// A source format was requested that 2.0-D does
    /// not support. 2.0-D only supports 3.0 (quilt);
    /// the variant carries the requested format name
    /// for diagnostics.
    #[error("source format {0} is not supported in 2.0-D (quilt only)")]
    UnsupportedSourceFormat(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{
        DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId,
        JobId, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
        VcsKind,
    };
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

    fn tarball() -> UpstreamTarball {
        UpstreamTarball {
            path: PathBuf::from("/var/lib/ironmaint/fixtures/hello_2.10.orig.tar.gz"),
            sha256: [0xab; 32],
        }
    }

    #[test]
    fn quilt_descriptor_carries_candidate_and_tarball() {
        let c = candidate();
        let t = tarball();
        let d = BuildInputDescriptor::quilt(c.clone(), t.clone());
        assert_eq!(d.candidate, c);
        assert_eq!(d.source_format.upstream_tarball, t);
    }

    #[test]
    fn upstream_tarball_filename_is_path_basename() {
        let t = UpstreamTarball {
            path: PathBuf::from("/var/lib/ironmaint/fixtures/hello_2.10.orig.tar.gz"),
            sha256: [0; 32],
        };
        assert_eq!(t.filename(), "hello_2.10.orig.tar.gz");
    }

    #[test]
    fn upstream_tarball_filename_falls_back_for_bare_path() {
        // A path with no file_name component (e.g. a
        // bare "upstream.tar.gz" that somehow lost its
        // dir) falls back to a sentinel so the staging
        // primitive has something deterministic to
        // write.
        let t = UpstreamTarball {
            path: PathBuf::from("upstream.tar.gz"),
            sha256: [0; 32],
        };
        assert_eq!(t.filename(), "upstream.tar.gz");
    }

    #[test]
    fn build_input_error_displays_mismatch() {
        let e = BuildInputError::UpstreamTarballSha256Mismatch {
            expected: "abc".into(),
            actual: "def".into(),
        };
        let s = e.to_string();
        assert!(s.contains("expected abc"));
        assert!(s.contains("got def"));
    }

    #[test]
    fn build_input_error_displays_unsupported_format() {
        let e = BuildInputError::UnsupportedSourceFormat("native");
        let s = e.to_string();
        assert!(s.contains("native"));
        assert!(s.contains("quilt only"));
    }
}
