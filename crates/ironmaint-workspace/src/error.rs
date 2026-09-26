//! Workspace error types.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkspaceErrorKind {
    #[error("path escapes workspace root (attempted: {0:?})")]
    PathEscapes(PathBuf),
    #[error("absolute paths are not allowed in workspace input")]
    AbsolutePath,
    #[error("workspace not found: {0:?}")]
    NotFound(PathBuf),
    #[error("optimistic concurrency conflict (expected {expected}, found {found})")]
    Conflict { expected: u64, found: u64 },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("external command failed: {0}")]
    Command(String),
    #[error("other: {0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct WorkspaceError {
    kind: WorkspaceErrorKind,
}

impl WorkspaceError {
    pub fn new(kind: WorkspaceErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> &WorkspaceErrorKind {
        &self.kind
    }
}

impl From<WorkspaceErrorKind> for WorkspaceError {
    fn from(kind: WorkspaceErrorKind) -> Self {
        Self { kind }
    }
}

impl From<std::io::Error> for WorkspaceError {
    fn from(e: std::io::Error) -> Self {
        Self {
            kind: WorkspaceErrorKind::Io(e),
        }
    }
}
