//! `ironmaint-artifacts` — content-addressed artifact store.
//!
//! Files are stored under `<root>/<aa>/<bb>/<full-hex>` where
//! `<aa>` and `<bb>` are the first two hex-byte pairs of the
//! SHA-256 digest (the "sharded by prefix" layout).
//!
//! Writes are atomic: stream → tempfile → hash → rename →
//! fsync parent dir. If the process dies mid-write the tempfile
//! is the only artifact left behind, and can be GC'd on next
//! startup; the eventual location is either non-existent or
//! complete.
//!
//! Reads verify the digest against the path component and refuse
//! to return bytes that do not match.
//!
//! All operations are async (PHASE-0B.md §12).

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

pub mod hash;
pub mod path;
pub mod reader;
pub mod store;
pub mod writer;

pub use path::ArtifactRoot;
pub use store::{ArtifactStore, Record};
