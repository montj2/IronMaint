//! Path confinement tests for `WorkspacePath::resolve`.
//!
//! Covers the §18 attack vectors:
//!
//! * Absolute paths (`/etc/passwd`).
//! * Parent traversal that escapes the root
//!   (`../../etc/passwd`, trailing `..`).
//! * Internal parent traversal that stays inside
//!   (`a/../b` → `b`).
//! * Stray current-dir segments (`./a/./b` → `a/b`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_workspace::{WorkspacePath, error::WorkspaceErrorKind, path_confinement_root};

#[test]
fn rejects_absolute_path() {
    let root = path_confinement_root();
    let err = WorkspacePath::resolve(&root, std::path::Path::new("/etc/passwd")).unwrap_err();
    assert!(matches!(err.kind(), WorkspaceErrorKind::AbsolutePath));
}

#[test]
fn rejects_two_dot_traversal() {
    let root = path_confinement_root();
    let err = WorkspacePath::resolve(&root, std::path::Path::new("../../etc/passwd")).unwrap_err();
    assert!(matches!(err.kind(), WorkspaceErrorKind::PathEscapes(_)));
}

#[test]
fn rejects_exact_parent() {
    let root = path_confinement_root();
    let err = WorkspacePath::resolve(&root, std::path::Path::new("..")).unwrap_err();
    assert!(matches!(err.kind(), WorkspaceErrorKind::PathEscapes(_)));
}

#[test]
fn keeps_internal_parent_inside() {
    let root = path_confinement_root();
    let p = WorkspacePath::resolve(&root, std::path::Path::new("a/../b")).unwrap();
    assert_eq!(p.relative.to_str().unwrap(), "b");
}

#[test]
fn drops_current_segments() {
    let root = path_confinement_root();
    let p = WorkspacePath::resolve(&root, std::path::Path::new("./a/./b")).unwrap();
    assert_eq!(p.relative.to_str().unwrap(), "a/b");
}
