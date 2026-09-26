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

/// `resolve_strict_no_follow` rejects a symlink component,
/// preventing writes from being redirected outside the root
/// (PHASE-0B.md §19).
#[test]
fn symlink_component_rejected_by_resolve_strict() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    // Place an actual file outside the root.
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, b"secret").unwrap();
    // Make a symlink inside root pointing to the outside file.
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

    let err =
        WorkspacePath::resolve_strict_no_follow(&root, std::path::Path::new("link")).unwrap_err();
    assert!(
        matches!(err.kind(), WorkspaceErrorKind::SymlinkPolicy(_)),
        "expected SymlinkPolicy, got {err:?}"
    );
}

/// `resolve_for_read` permits following an in-workspace symlink
/// to a real file inside the root.
#[test]
fn in_root_symlink_resolves_for_read() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    // Create a real file `a` inside root and a symlink `link → a`.
    std::fs::write(root.join("a"), b"hello").unwrap();
    std::os::unix::fs::symlink("a", root.join("link")).unwrap();

    let p = WorkspacePath::resolve_for_read(&root, std::path::Path::new("link")).unwrap();
    // The full path is `<root>/link`; the canonical path
    // would resolve to `<root>/a`, which is still inside root.
    let canon = std::fs::canonicalize(p.full()).unwrap();
    let root_canon = std::fs::canonicalize(&root).unwrap();
    assert!(canon.starts_with(&root_canon));
}

/// `resolve_for_read` rejects an in-workspace symlink whose
/// target is outside the root.
#[test]
fn escape_symlink_rejected_by_resolve_for_read() {
    let tmp = tempfile::tempdir().unwrap();
    // Place the workspace root INSIDE a parent dir so we can put
    // the escape target in a sibling location that is genuinely
    // outside the root.
    let parent = tmp.path();
    let root = parent.join("ws-root");
    std::fs::create_dir(&root).unwrap();
    let outside = parent.join("outside.txt");
    std::fs::write(&outside, b"secret").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

    let err = WorkspacePath::resolve_for_read(&root, std::path::Path::new("link")).unwrap_err();
    assert!(
        matches!(err.kind(), WorkspaceErrorKind::SymlinkPolicy(_)),
        "expected SymlinkPolicy, got {err:?}"
    );
}
