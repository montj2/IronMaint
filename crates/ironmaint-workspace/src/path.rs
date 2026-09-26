//! Path confinement for workspace-relative operations.
//!
//! A `WorkspacePath` is *always* an absolute path obtained by
//! resolving a relative user-supplied input against the
//! workspace root. The resolver rejects:
//!
//! * Absolute paths (`/etc/passwd`).
//! * Parent traversal that leaves the root
//!   (`../../etc/passwd`).
//! * Empty segments left by stray slashes (`a//b` becomes
//!   `a/b`; PHASE-0B.md §18).
//!
//! `path_confinement_root` constructs a `Path` against an
//! in-memory root so the resolver can be exercised without
//! hitting the filesystem.

use std::path::{Component, Path, PathBuf};

use crate::error::{WorkspaceError, WorkspaceErrorKind};

#[derive(Debug, Clone)]
pub struct WorkspacePath {
    pub root: PathBuf,
    pub relative: PathBuf,
}

impl WorkspacePath {
    /// Try to resolve `relative` against `root`.
    ///
    /// Returns `WorkspaceError::PathEscapes` if the resolved
    /// path escapes the root, and `WorkspaceError::AbsolutePath`
    /// if `relative` is absolute.
    pub fn resolve(root: &Path, relative: &Path) -> Result<Self, WorkspaceError> {
        if relative.is_absolute() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::AbsolutePath));
        }
        let mut clean = PathBuf::new();
        for comp in relative.components() {
            match comp {
                Component::Normal(seg) => clean.push(seg),
                Component::CurDir => {}
                Component::ParentDir => {
                    if clean.pop() {
                        // popped OK, still inside root
                    } else {
                        return Err(WorkspaceError::new(WorkspaceErrorKind::PathEscapes(
                            relative.to_path_buf(),
                        )));
                    }
                }
                Component::Prefix(_) | Component::RootDir => {
                    return Err(WorkspaceError::new(WorkspaceErrorKind::AbsolutePath));
                }
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            relative: clean,
        })
    }

    /// Re-attach the cleaned relative path back to the root.
    pub fn full(&self) -> PathBuf {
        self.root.join(&self.relative)
    }

    /// Resolve `relative` against `root` with a strict no-follow
    /// symlink policy (PHASE-0B.md §19). Each component of the
    /// cleaned path is `symlink_metadata`-checked; any component
    /// whose type is a symlink is rejected with
    /// `WorkspaceErrorKind::SymlinkPolicy`. Used by write paths
    /// (apply_patch, capture_candidate, future commit flows)
    /// where the workspace must not be coaxed into writing
    /// through a symlink that escapes the root.
    pub fn resolve_strict_no_follow(root: &Path, relative: &Path) -> Result<Self, WorkspaceError> {
        let cleaned = Self::resolve(root, relative)?;
        // Walk each component under the root and check the
        // type. A component that is a symlink is rejected.
        let mut acc = PathBuf::new();
        for comp in cleaned.relative.components() {
            if let Component::Normal(seg) = comp {
                acc.push(seg);
                let candidate = cleaned.root.join(&acc);
                let md = std::fs::symlink_metadata(&candidate)
                    .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Io(e)))?;
                if md.file_type().is_symlink() {
                    return Err(WorkspaceError::new(WorkspaceErrorKind::SymlinkPolicy(
                        format!("component `{}` is a symlink", acc.display()),
                    )));
                }
            }
        }
        Ok(cleaned)
    }

    /// Resolve `relative` against `root` with a read-friendly
    /// symlink policy (PHASE-0B.md §19). Symlinks may be followed,
    /// but the resulting canonical path must still be inside the
    /// root — otherwise the resolution escapes and the call is
    /// rejected with `WorkspaceErrorKind::SymlinkPolicy`. Used by
    /// read/list/status paths where following legitimate
    /// in-workspace symlinks (e.g., `link → dir/`) is desired
    /// but escape attempts are blocked.
    pub fn resolve_for_read(root: &Path, relative: &Path) -> Result<Self, WorkspaceError> {
        let cleaned = Self::resolve(root, relative)?;
        let candidate = cleaned.full();
        // canonicalize resolves every symlink in the path. If the
        // result escapes `root`, the call is a symlink-escape
        // attempt.
        let canon = match std::fs::canonicalize(&candidate) {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Not-found is not a symlink policy violation; the
                // read path will produce its own not-found error
                // when it tries to open the file.
                return Ok(cleaned);
            }
            Err(e) => return Err(WorkspaceError::new(WorkspaceErrorKind::Io(e))),
        };
        if !canon.starts_with(cleaned.root.canonicalize().unwrap_or(cleaned.root.clone())) {
            return Err(WorkspaceError::new(WorkspaceErrorKind::SymlinkPolicy(
                format!(
                    "canonical path `{}` escapes root `{}`",
                    canon.display(),
                    cleaned.root.display()
                ),
            )));
        }
        Ok(cleaned)
    }
}

/// Construct a non-existing path used for unit tests that exercise
/// the resolver in isolation.
pub fn path_confinement_root() -> PathBuf {
    std::env::temp_dir().join("ironmaint-workspace-confinement-test")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        path_confinement_root()
    }

    #[test]
    fn relative_path_resolves() {
        let p = WorkspacePath::resolve(&root(), Path::new("a/b/c")).unwrap();
        assert_eq!(p.relative.to_str().unwrap(), "a/b/c");
    }

    #[test]
    fn absolute_path_rejected() {
        let err = WorkspacePath::resolve(&root(), Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err.kind(), WorkspaceErrorKind::AbsolutePath));
    }

    #[test]
    fn parent_traversal_rejected() {
        let err = WorkspacePath::resolve(&root(), Path::new("../../etc/passwd")).unwrap_err();
        assert!(matches!(err.kind(), WorkspaceErrorKind::PathEscapes(_)));
    }

    #[test]
    fn exact_parent_rejected() {
        let err = WorkspacePath::resolve(&root(), Path::new("..")).unwrap_err();
        assert!(matches!(err.kind(), WorkspaceErrorKind::PathEscapes(_)));
    }

    #[test]
    fn internal_parent_resolves_inside() {
        let p = WorkspacePath::resolve(&root(), Path::new("a/../b")).unwrap();
        assert_eq!(p.relative.to_str().unwrap(), "b");
    }

    #[test]
    fn current_segment_drops() {
        let p = WorkspacePath::resolve(&root(), Path::new("./a/./b")).unwrap();
        assert_eq!(p.relative.to_str().unwrap(), "a/b");
    }
}
