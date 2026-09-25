//! Content digests (§11).
//!
//! Two related types live here:
//!
//! - [`Digest`] — the general digest, with one of several
//!   [`DigestAlgorithm`] variants. Use this for *artifacts* and any
//!   general hash reference. Artifacts default to SHA-256
//!   (`PHASE-0A.md §11: "Artifacts use SHA-256"`).
//! - [`GitObjectId`] — specifically for Git object identities.
//!   Carries its own [`GitHashAlgorithm`] enum so the wire shape
//!   distinguishes "this is a Git object hash, not an arbitrary
//!   digest".

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Hex length of a BLAKE3-256 digest. (256 bits / 4 bits per hex char.)
const HEX_LEN_BLAKE3_SHA256_GIT_SHA256: usize = 64;
/// Hex length of a Git SHA-1 digest.
const HEX_LEN_GIT_SHA1: usize = 40;

fn validate_hex(field: &str, value: &str, expected_len: usize) -> Result<(), CoreError> {
    if value.len() != expected_len {
        return Err(CoreError::invalid_digest(format!(
            "{field} hex length must be {expected_len}, got {}",
            value.len()
        )));
    }
    if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CoreError::invalid_digest(format!(
            "{field} contains non-hex characters: `{value}`"
        )));
    }
    Ok(())
}

/// Digest algorithm used by [`Digest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestAlgorithm {
    /// BLAKE3-256. Default for `CandidateFingerprint` (PHASE-0A.md §12).
    Blake3,
    /// SHA-256. Default for artifact digests (PHASE-0A.md §11).
    Sha256,
    /// Git SHA-1. Preserved when a digest points at a Git object
    /// address from a legacy SHA-1 repository.
    GitSha1,
    /// Git SHA-256. Preserved when a digest points at a Git object
    /// address from a SHA-256 repository.
    GitSha256,
}

impl DigestAlgorithm {
    /// Expected hex length for this algorithm's digest.
    #[must_use]
    pub const fn hex_length(self) -> usize {
        match self {
            Self::Blake3 | Self::Sha256 | Self::GitSha256 => HEX_LEN_BLAKE3_SHA256_GIT_SHA256,
            Self::GitSha1 => HEX_LEN_GIT_SHA1,
        }
    }

    /// Human-readable name for the algorithm.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Blake3 => "blake3",
            Self::Sha256 => "sha256",
            Self::GitSha1 => "git-sha1",
            Self::GitSha256 => "git-sha256",
        }
    }
}

impl std::fmt::Display for DigestAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Content digest: `{ algorithm, hex_value }`.
///
/// `value` must be the lowercase (or uppercase) hexadecimal string of
/// length matching `algorithm.hex_length()`. Both cases are accepted
/// at construction; the canonical lowercase form is stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Digest {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}

impl Digest {
    /// Construct a `Digest` from a hex string for the given algorithm.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidDigest, .. }` if the hex
    /// length does not match the algorithm or contains non-hex bytes.
    pub fn new(algorithm: DigestAlgorithm, value: impl Into<String>) -> Result<Self, CoreError> {
        let v = value.into();
        validate_hex(algorithm.name(), &v, algorithm.hex_length())?;
        Ok(Self {
            algorithm,
            value: v.to_ascii_lowercase(),
        })
    }

    /// Borrow the hex value (canonical lowercase).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    #[must_use]
    pub fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }
}

/// Git-specific hash algorithm.
///
/// Used only by [`GitObjectId`]. Kept distinct from
/// [`DigestAlgorithm`] because the wire shape for a Git object
/// reference carries different semantics than a generic artifact
/// digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHashAlgorithm {
    Sha1,
    Sha256,
}

impl GitHashAlgorithm {
    #[must_use]
    pub const fn hex_length(self) -> usize {
        match self {
            Self::Sha256 => HEX_LEN_BLAKE3_SHA256_GIT_SHA256,
            Self::Sha1 => HEX_LEN_GIT_SHA1,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
        }
    }
}

impl std::fmt::Display for GitHashAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Git object identifier.
///
/// The shape (`algorithm` + `hex_value`) is the same as [`Digest`],
/// but the algorithm enum is narrower: only the two algorithms Git
/// actually uses today.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GitObjectId {
    pub algorithm: GitHashAlgorithm,
    pub value: String,
}

impl GitObjectId {
    /// Construct a `GitObjectId` from a hex string for the given
    /// Git hash algorithm.
    ///
    /// # Errors
    /// Same as [`Digest::new`].
    pub fn new(algorithm: GitHashAlgorithm, value: impl Into<String>) -> Result<Self, CoreError> {
        let v = value.into();
        validate_hex(algorithm.name(), &v, algorithm.hex_length())?;
        Ok(Self {
            algorithm,
            value: v.to_ascii_lowercase(),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Convenience: borrow the hex value (used by
    /// `SourceCandidate` fingerprinting).
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    #[must_use]
    pub fn algorithm(&self) -> GitHashAlgorithm {
        self.algorithm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreErrorKind;

    #[test]
    fn digest_blake3_accepts_64_hex_chars() {
        let hex = "a".repeat(64);
        let d = Digest::new(DigestAlgorithm::Blake3, hex).unwrap();
        assert_eq!(d.as_str().len(), 64);
    }

    #[test]
    fn digest_sha256_rejects_wrong_length() {
        let hex = "a".repeat(63);
        let err = Digest::new(DigestAlgorithm::Sha256, hex).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidDigest);
    }

    #[test]
    fn digest_git_sha1_accepts_40_hex_chars() {
        let hex = "0".repeat(40);
        let d = Digest::new(DigestAlgorithm::GitSha1, hex).unwrap();
        assert_eq!(d.as_str().len(), 40);
    }

    #[test]
    fn digest_rejects_non_hex() {
        let hex = "z".repeat(64);
        let err = Digest::new(DigestAlgorithm::Blake3, hex).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidDigest);
    }

    #[test]
    fn digest_normalizes_to_lowercase() {
        let d = Digest::new(DigestAlgorithm::Sha256, "A".repeat(64)).unwrap();
        assert!(
            d.as_str()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn digest_serializes_with_snake_case_algorithm() {
        let d = Digest::new(DigestAlgorithm::GitSha1, "0".repeat(40)).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["algorithm"], "git_sha1");
    }

    #[test]
    fn git_object_id_sha1_accepts_40_chars() {
        let id = GitObjectId::new(GitHashAlgorithm::Sha1, "0".repeat(40)).unwrap();
        assert_eq!(id.value(), "0".repeat(40).as_str());
    }

    #[test]
    fn git_object_id_sha256_accepts_64_chars() {
        let id = GitObjectId::new(GitHashAlgorithm::Sha256, "1".repeat(64)).unwrap();
        assert_eq!(id.value(), "1".repeat(64).as_str());
    }

    #[test]
    fn git_object_id_rejects_wrong_length() {
        let err = GitObjectId::new(GitHashAlgorithm::Sha1, "0".repeat(41)).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidDigest);
    }
}
