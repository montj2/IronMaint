//! SHA-256 streaming hasher.
//!
//! Wraps `sha2::Sha256` so the rest of the crate doesn't import
//! the algorithm crate directly. The `Sha256Of` newtype carries
//! the hex digest around for type-safe handoff between writer
//! and reader.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Lowercase hex string of a SHA-256 digest (64 chars).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Compute the SHA-256 of `bytes` in one shot.
pub fn sha256_of_bytes(bytes: &[u8]) -> Sha256Hex {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Sha256Hex(hex_lower(&hasher.finalize()))
}

/// Streaming hasher. Use `update` to absorb data, then `finalize`
/// to consume the hasher into the digest.
#[derive(Default)]
pub struct Sha256Writer {
    inner: Sha256,
}

impl Sha256Writer {
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    pub fn finalize(self) -> Sha256Hex {
        Sha256Hex(hex_lower(&self.inner.finalize()))
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
