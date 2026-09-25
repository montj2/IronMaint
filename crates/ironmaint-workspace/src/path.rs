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
