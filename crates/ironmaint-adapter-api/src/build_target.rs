//! `BuildTarget` — per-candidate (suite, arch) for build invocation.
//!
//! PHASE-2 PR 2.0-C (lock the build execution model,
//! `doc/PHASE-2-C.md`). The Debian adapter's build
//! invocation targets a specific Debian suite
//! (`sid`, `bookworm`, etc.) and a specific Debian
//! architecture (`amd64`, `arm64`, etc.). For the
//! first slice, the per-candidate default is
//! `(suite="sid", arch=<host arch>)`; the 2A.2 PR
//! adds per-candidate overrides and multi-arch.
//!
//! This module is the spec for the (suite, arch)
//! type. The [`BuildTargetProvider`] trait is the
//! opt-in capability an adapter implements to
//! publish its default. The runtime integration
//! (asking the adapter for the target per
//! candidate) lands in 2A.x; 2.0-C only defines
//! the type, the defaults, and the validation
//! rules.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contexts::CandidateContext;
use crate::error::{AdapterError, AdapterErrorKind};

/// A build target: the Debian suite to build for
/// and the Debian architecture to build for.
///
/// `suite` is the release name (e.g. `sid`,
/// `bookworm`, `trixie`). `arch` is the Debian
/// architecture name (e.g. `arm64`, `amd64`). The
/// [`parse`](Self::parse) constructor validates
/// both; [`default_for_candidate`](Self::default_for_candidate)
/// returns the per-candidate default for the
/// first slice.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct BuildTarget {
    pub suite: String,
    pub arch: String,
}

impl BuildTarget {
    /// The first-slice per-candidate default. The
    /// Debian suite is `sid` (the moving target;
    /// matches the IronMaint fixtures'
    /// `DistributionRelease::new("sid")` choice,
    /// per `adapters/debian-stub/src/release.rs`
    /// and `crates/ironmaint-policy/src/policy.rs`).
    /// The architecture is the binary's compile-time
    /// host architecture, mapped to the Debian
    /// name (`aarch64 → arm64`, `x86_64 → amd64`).
    ///
    /// The `ctx` argument is currently unused but
    /// is part of the signature so 2A.2 can read
    /// the candidate's per-package override
    /// without a breaking change.
    ///
    /// 2A.2 will add per-candidate overrides (e.g.
    /// `Build-Target: trixie/arm64` in the
    /// candidate's `debian/control`-equivalent
    /// metadata).
    #[must_use]
    pub fn default_for_candidate(_ctx: &CandidateContext<'_>) -> Self {
        Self {
            suite: "sid".to_string(),
            arch: host_arch(),
        }
    }

    /// Parse a `(suite, arch)` pair from operator
    /// input. Both fields are validated: the suite
    /// must be a non-empty `[a-z0-9+.-]+` string
    /// (Debian suite names follow that pattern),
    /// and the arch must be in the allowlist of
    /// known Debian architectures.
    ///
    /// # Errors
    /// Returns `AdapterError { kind:
    /// InvalidConfiguration, .. }` for either
    /// rejection.
    pub fn parse(suite: &str, arch: &str) -> Result<Self, AdapterError> {
        if suite.is_empty() {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                "build target suite is empty",
            ));
        }
        if !suite
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                format!(
                    "build target suite {suite:?} contains non-suite characters; \
                     Debian suite names match [a-z0-9+.-]+"
                ),
            ));
        }
        if !is_known_debian_arch(arch) {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                format!(
                    "build target arch {arch:?} is not a known Debian architecture; \
                     supported: {}",
                    KNOWN_DEBIAN_ARCHES.join(", ")
                ),
            ));
        }
        Ok(Self {
            suite: suite.to_string(),
            arch: arch.to_string(),
        })
    }
}

impl fmt::Display for BuildTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.suite, self.arch)
    }
}

/// Map the compile-time host architecture
/// (`std::env::consts::ARCH`) to its Debian
/// architecture name. Returns
/// `"unknown-<arch>"` for arches the workspace
/// has not registered; the operator can still
/// pass an explicit target via
/// [`BuildTarget::parse`].
///
/// The mapping is a one-way lookup table; we
/// don't try to be clever about cross-compile
/// targets — the first slice is build-on-host.
#[must_use]
pub fn host_arch() -> String {
    match std::env::consts::ARCH {
        "aarch64" => "arm64".to_string(),
        "x86_64" => "amd64".to_string(),
        "i686" => "i386".to_string(),
        "arm" => "armhf".to_string(),
        "powerpc64" => "ppc64el".to_string(),
        "riscv64" => "riscv64".to_string(),
        "s390x" => "s390x".to_string(),
        other => format!("unknown-{other}"),
    }
}

/// The Debian architectures the workspace knows
/// about. Aligned with `dpkg --print-architecture`
/// outputs for the targets we care about; new
/// entries land here as the workspace's
/// target-arch matrix grows.
const KNOWN_DEBIAN_ARCHES: &[&str] = &[
    "amd64", "arm64", "armel", "armhf", "i386", "mips", "mips64el", "mipsel", "ppc64el", "s390x",
    "riscv64",
];

fn is_known_debian_arch(arch: &str) -> bool {
    KNOWN_DEBIAN_ARCHES.contains(&arch)
}

/// Optional capability: an adapter publishes the
/// per-candidate build target it wants invoked.
///
/// Adapters that do not implement this trait do
/// not have a per-candidate build target; the
/// runtime falls back to
/// [`BuildTarget::default_for_candidate`]
/// (which still uses the host arch, but the
/// suite defaulting is the adapter's call).
///
/// The Debian adapter (production) and the
/// Debian stub both implement this. The Fedora
/// stub also implements it (the Fedora follow-on
/// in Phase 2 will refine the default from
/// `sid` to `rawhide`).
pub trait BuildTargetProvider: Send + Sync + 'static {
    /// The per-candidate build target this
    /// adapter wants.
    fn build_target(&self, ctx: &CandidateContext<'_>) -> BuildTarget;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{
        DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId,
        JobId, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
        SourceCandidate, VcsKind,
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
    fn host_arch_is_recognised_on_workspace_targets() {
        // The workspace pins the build host to one of
        // two architectures (aarch64 on Apple Silicon,
        // x86_64 on Intel); the other registered
        // arches are there for completeness and would
        // only be hit by a cross-build.
        let arch = host_arch();
        assert!(
            is_known_debian_arch(&arch),
            "the compile-time host arch {arch:?} (std::env::consts::ARCH = {:?}) \
             does not map to a known Debian architecture; add the mapping in \
             `host_arch()`",
            std::env::consts::ARCH
        );
    }

    #[test]
    fn default_for_candidate_is_sid_plus_host_arch() {
        let c = candidate();
        let ctx = CandidateContext {
            package: c.package(),
            candidate: &c,
        };
        let t = BuildTarget::default_for_candidate(&ctx);
        assert_eq!(t.suite, "sid");
        assert_eq!(t.arch, host_arch());
    }

    #[test]
    fn parse_accepts_known_pair() {
        let t = BuildTarget::parse("sid", "arm64").unwrap();
        assert_eq!(t.suite, "sid");
        assert_eq!(t.arch, "arm64");
    }

    #[test]
    fn parse_accepts_bookworm_and_trixie() {
        // Verify the validator's character class is
        // correct for real suite names (the
        // post-`bookworm` era uses `bookworm`,
        // `trixie`, `forky`, `sid`).
        let t = BuildTarget::parse("trixie", "amd64").unwrap();
        assert_eq!(t.suite, "trixie");
    }

    #[test]
    fn parse_rejects_empty_suite() {
        let err = BuildTarget::parse("", "arm64").unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn parse_rejects_suite_with_spaces() {
        let err = BuildTarget::parse("my suite", "arm64").unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn parse_rejects_unknown_arch() {
        let err = BuildTarget::parse("sid", "m68k").unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn display_is_suite_slash_arch() {
        let t = BuildTarget {
            suite: "sid".into(),
            arch: "arm64".into(),
        };
        assert_eq!(format!("{t}"), "sid/arm64");
    }

    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn BuildTargetProvider>();
    }
}
