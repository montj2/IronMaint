//! `BuildInputHandle` — the disposable, staged source tree
//! a real build tool sees.
//!
//! PHASE-2 PR 2.0-D (make the candidate/build boundary
//! explicit, `doc/PHASE-2-D.md`). A `BuildInputHandle` is
//! the *where* a build runs from. It is **not** a
//! `FrozenWorktree` (which is the read-only *check* view);
//! it is a sibling type, with a different filesystem root
//! and a different purpose.
//!
//! ## Layout
//!
//! ```text
//! <root>/<handle>/build-input/<fingerprint>/
//!     <files extracted from the candidate tree>
//!     <upstream tarball, basename of the descriptor's path>
//! <root>/<handle>/build-output/<fingerprint>/
//!     (empty; created at stage time for the executor to use)
//! ```
//!
//! The staged root is `chmod -R a-w`'d at stage time
//! (PR §2.0-D bullet 5: the candidate is immutable, the
//! build is the only writer, and writers go to
//! `output_dir`, never to the staged root).
//!
//! ## Lifetime
//!
//! The handle is the executor's proof that the staged
//! tree is the candidate's tree (the `tree_sha` accessor
//! returns the re-derived OID). The handle's `Drop` impl
//! removes the staged root and the output dir on best
//! effort; an executor that holds a `BuildInputHandle`
//! across calls has a contract that says "the handle's
//! path is valid for the duration of the call." Outside
//! the call, the path is fair game for the next staging.

use std::path::{Path, PathBuf};

use ironmaint_adapter_api::SourceFormatQuilt;
use ironmaint_core::SourceCandidate;

/// The staged build input. See module docs for layout.
///
/// `root` is the staged source tree (read-only after
/// staging; the build tool sees this path). `tree_sha` is
/// the re-derived OID of the staged tree, asserted equal
/// to `candidate.tree()` at stage time. `output_dir` is
/// a sibling of `root`; the build tool's CWD is set here
/// via `with_env`'s `BUILD_OUTPUT_DIR` override (2A.1
/// wires the env).
#[derive(Debug, Clone)]
pub struct BuildInputHandle {
    /// The staged source tree. Read-only after staging.
    root: PathBuf,
    /// Re-derived tree OID. Equal to
    /// `candidate.tree()` by construction.
    tree_sha: String,
    /// The candidate this handle staged. Carries
    /// `fingerprint`, `commit`, `tree` for the executor to
    /// bind into the `BuildReport`.
    candidate: SourceCandidate,
    /// `SourceFormatQuilt { upstream_tarball }`. Carried
    /// so the executor can read the upstream tarball's
    /// filename without re-reading the descriptor.
    format: SourceFormatQuilt,
    /// The upstream tarball's path inside the staged
    /// root. Equal to `root.join(format.upstream_tarball.filename())`.
    upstream_tarball_path: PathBuf,
    /// The output directory. Sibling of `root`, not a
    /// subdirectory.
    output_dir: PathBuf,
}

impl BuildInputHandle {
    /// Construct a handle. This is the workspace's
    /// internal constructor; callers outside the crate
    /// use [`crate::WorkspaceManager::stage_build_input`].
    ///
    /// The 2.0-D RED commit's `WorkspaceManager::stage_build_input`
    /// is a stub that does not call this; the GREEN
    /// commit replaces the stub and uses it. The
    /// `#[allow(dead_code)]` keeps the workspace's
    /// `cargo build` green while the stub is in place.
    #[must_use]
    #[allow(dead_code, clippy::too_many_arguments)]
    pub(crate) fn new(
        root: PathBuf,
        tree_sha: String,
        candidate: SourceCandidate,
        format: SourceFormatQuilt,
        upstream_tarball_path: PathBuf,
        output_dir: PathBuf,
    ) -> Self {
        Self {
            root,
            tree_sha,
            candidate,
            format,
            upstream_tarball_path,
            output_dir,
        }
    }

    /// The staged source tree. Read-only.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The re-derived tree OID. Equal to
    /// `candidate.tree()` by construction.
    #[must_use]
    pub fn tree_sha(&self) -> &str {
        &self.tree_sha
    }

    /// The candidate this handle staged. Carries
    /// `fingerprint`, `commit`, `tree` for the executor
    /// to bind into the `BuildReport`.
    #[must_use]
    pub fn candidate(&self) -> &SourceCandidate {
        &self.candidate
    }

    /// The source format (3.0 quilt, for 2.0-D).
    #[must_use]
    pub fn format(&self) -> &SourceFormatQuilt {
        &self.format
    }

    /// The path of the upstream tarball inside the
    /// staged root. Equal to
    /// `root.join(format.upstream_tarball.filename())`.
    #[must_use]
    pub fn upstream_tarball_path(&self) -> &Path {
        &self.upstream_tarball_path
    }

    /// The output directory. Sibling of `root`, not a
    /// subdirectory. The build tool's CWD is set here.
    #[must_use]
    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    /// Whether the staged root is read-only. The
    /// staging primitive `chmod -R a-w`'s the staged
    /// root after the `git write-tree` re-derivation
    /// succeeds; the executor uses this as a
    /// precondition check before running the build.
    #[must_use]
    pub fn is_staged_root_readonly(&self) -> bool {
        // The chmod is at stage time; the read-only state
        // is therefore a property of the filesystem, not
        // the handle. The accessor delegates to a
        // `metadata().permissions().readonly()` check on
        // the staged root.
        std::fs::metadata(&self.root)
            .map(|m| m.permissions().readonly())
            .unwrap_or(false)
    }
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

    #[test]
    fn accessors_return_what_the_constructor_stored() {
        let c = candidate();
        let format = SourceFormatQuilt {
            upstream_tarball: ironmaint_adapter_api::UpstreamTarball {
                path: PathBuf::from("/var/lib/ironmaint/fixtures/hello_2.10.orig.tar.gz"),
                sha256: [0xab; 32],
            },
        };
        let h = BuildInputHandle::new(
            PathBuf::from("/tmp/staged"),
            hex('b', 40),
            c.clone(),
            format.clone(),
            PathBuf::from("/tmp/staged/hello_2.10.orig.tar.gz"),
            PathBuf::from("/tmp/output"),
        );
        assert_eq!(h.root(), Path::new("/tmp/staged"));
        assert_eq!(h.tree_sha(), hex('b', 40));
        assert_eq!(h.candidate(), &c);
        assert_eq!(h.format(), &format);
        assert_eq!(
            h.upstream_tarball_path(),
            Path::new("/tmp/staged/hello_2.10.orig.tar.gz")
        );
        assert_eq!(h.output_dir(), Path::new("/tmp/output"));
    }

    #[test]
    fn is_staged_root_readonly_returns_false_for_missing_path() {
        // A handle whose root does not exist on disk
        // (the test never creates it) reports
        // `is_staged_root_readonly() == false`. The
        // accessor is a precondition check, not a
        // invariant — a missing root is not a
        // permission error.
        let c = candidate();
        let format = SourceFormatQuilt {
            upstream_tarball: ironmaint_adapter_api::UpstreamTarball {
                path: PathBuf::from("/var/lib/ironmaint/fixtures/hello_2.10.orig.tar.gz"),
                sha256: [0; 32],
            },
        };
        let h = BuildInputHandle::new(
            PathBuf::from("/this/path/does/not/exist"),
            hex('b', 40),
            c,
            format,
            PathBuf::from("/this/path/does/not/exist/hello_2.10.orig.tar.gz"),
            PathBuf::from("/this/output/does/not/exist"),
        );
        assert!(!h.is_staged_root_readonly());
    }
}
