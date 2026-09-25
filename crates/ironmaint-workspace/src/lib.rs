//! `ironmaint-workspace` — per-job workspace manager.
//!
//! Each job gets an isolated working tree under the workspace root.
//! Path resolution is *confined*: `..` segments and absolute paths
//! in user input are rejected before they can escape the root
//! (PHASE-0B.md §18).
//!
//! `WorkspaceRevision` is a u64 newtype held in core of the
//! workspace crate (NOT `ironmaint-core`): the revision only has
//! meaning in the context of an editor/manager and is not a
//! domain concept. `WorkspaceMetadataStore` carries the persisted
//! `(handle → revision)` mapping, and `apply_patch` uses an
//! expected-revision CAS to detect concurrent edits.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
    )
)]

pub mod apply;
pub mod error;
pub mod git;
pub mod id;
pub mod manager;
pub mod path;
pub mod revision;

pub use error::{WorkspaceError, WorkspaceErrorKind};
pub use id::WorkspaceId;
pub use manager::WorkspaceManager;
pub use path::{WorkspacePath, path_confinement_root};
pub use revision::WorkspaceRevision;
