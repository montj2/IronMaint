//! Repository reference (§10).
//!
//! Core records *what* the canonical repository is (VCS kind + URL)
//! and *where* within that repository a file lives (relative path).
//! It never carries a host-local absolute path: that's the spec's
//! "Never persist host-local absolute paths as package identity" rule.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::CoreError;

const REPO_PATH_MAX: usize = 4096;

/// Version-control system a `RepositoryRef` is anchored in.
///
/// Phase 0A only carries `Git`. Additional variants land when an
/// adapter implements them — adding a variant does NOT require
/// changes to core code (the type is exhaustively switched only by
/// adapter capability ports, not by `ironmaint-core`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VcsKind {
    Git,
}

/// Canonical repository reference.
///
/// `canonical_url` is the URL the adapter considers authoritative
/// (e.g. `https://salsa.debian.org/foo/foo.git` for a Debian package
/// or `https://src.fedoraproject.org/rpms/foo.git` for a Fedora
/// package). The URL must have a scheme — relative URLs are
/// rejected because they can't be canonicalized.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct RepositoryRef {
    pub vcs: VcsKind,
    pub canonical_url: Url,
}

impl RepositoryRef {
    /// Construct a `RepositoryRef`.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidUrl, .. }` if the URL is
    /// relative (no scheme) or cannot form a base.
    pub fn new(vcs: VcsKind, canonical_url: Url) -> Result<Self, CoreError> {
        validate_url(&canonical_url)?;
        Ok(Self { vcs, canonical_url })
    }

    #[must_use]
    pub fn url(&self) -> &Url {
        &self.canonical_url
    }
}

fn validate_url(url: &Url) -> Result<(), CoreError> {
    if url.cannot_be_a_base() {
        return Err(CoreError::invalid_url(format!(
            "URL `{url}` cannot be a base (no scheme or unsupported scheme)"
        )));
    }
    if url.scheme().is_empty() {
        return Err(CoreError::invalid_url(format!("URL `{url}` has no scheme")));
    }
    Ok(())
}

/// Repository-relative path within a `RepositoryRef`.
///
/// Reject rules:
/// - empty
/// - leading `/` (host-absolute, not repository-relative)
/// - any `..` segment (path traversal)
/// - NUL byte
///
/// Maximum length 4096 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RepoPath(String);

impl RepoPath {
    /// Construct a `RepoPath`.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidPath, .. }` on any reject rule.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let s = value.into();
        validate_repo_path(&s)?;
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for RepoPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RepoPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for RepoPath {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

fn validate_repo_path(s: &str) -> Result<(), CoreError> {
    if s.is_empty() {
        return Err(CoreError::invalid_path("repo path must not be empty"));
    }
    if s.len() > REPO_PATH_MAX {
        return Err(CoreError::invalid_path(format!(
            "repo path length {} exceeds max {REPO_PATH_MAX}",
            s.len()
        )));
    }
    if s.starts_with('/') {
        return Err(CoreError::invalid_path(format!(
            "repo path `{s}` must not be absolute (no leading /)"
        )));
    }
    if s.bytes().any(|b| b == b'\0') {
        return Err(CoreError::invalid_path(
            "repo path contains NUL byte".to_string(),
        ));
    }
    // Reject any `..` segment at any depth. Spec says "no `..`" without
    // qualification; we apply the strict interpretation.
    for segment in s.split('/') {
        if segment == ".." {
            return Err(CoreError::invalid_path(format!(
                "repo path `{s}` contains `..` segment"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreErrorKind;

    fn git_url(s: &str) -> Url {
        Url::parse(s).expect("test fixture")
    }

    #[test]
    fn repo_ref_accepts_canonical_https() {
        let r = RepositoryRef::new(
            VcsKind::Git,
            git_url("https://salsa.debian.org/foo/foo.git"),
        );
        assert!(r.is_ok());
    }

    #[test]
    fn repo_ref_accepts_git_scheme() {
        let r = RepositoryRef::new(
            VcsKind::Git,
            git_url("git://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git"),
        );
        assert!(r.is_ok());
    }

    #[test]
    fn repo_ref_rejects_data_url() {
        let url = Url::parse("data:text/plain,foo").unwrap();
        let err = RepositoryRef::new(VcsKind::Git, url).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidUrl);
    }

    #[test]
    fn vcs_kind_serializes_as_snake_case() {
        let json = serde_json::to_string(&VcsKind::Git).unwrap();
        assert_eq!(json, "\"git\"");
    }

    #[test]
    fn repo_path_accepts_canonical_examples() {
        for p in [
            "debian/control",
            "foo.spec",
            "src/Makefile.am",
            "a/b/c/d.txt",
        ] {
            assert!(
                RepoPath::new(p).is_ok(),
                "expected `{p}` to be a valid repo path"
            );
        }
    }

    #[test]
    fn repo_path_rejects_empty() {
        let err = RepoPath::new("").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidPath);
    }

    #[test]
    fn repo_path_rejects_leading_slash() {
        let err = RepoPath::new("/etc/passwd").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidPath);
    }

    #[test]
    fn repo_path_rejects_dotdot_segment_at_root() {
        let err = RepoPath::new("../foo").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidPath);
    }

    #[test]
    fn repo_path_rejects_dotdot_segment_in_middle() {
        let err = RepoPath::new("a/b/../c").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidPath);
    }

    #[test]
    fn repo_path_rejects_nul_byte() {
        let err = RepoPath::new("foo\0bar").unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidPath);
    }

    #[test]
    fn repo_path_accepts_dot_segment() {
        // A single `.` segment is allowed (it's a relative-path notation
        // that doesn't escape). Only `..` is rejected.
        assert!(RepoPath::new("./foo").is_ok());
        assert!(RepoPath::new("a/./b").is_ok());
    }
}
