//! Artifact references (§25).
//!
//! An [`ArtifactRef`] is a pointer to bytes that live elsewhere —
//! filesystem, S3-compatible store, database blob, remote artifact
//! service. IronMaint carries the identity and integrity (digest)
//! of the artifact, not the artifact itself. Storage is the
//! adapter's concern (§25: "artifact bytes do not live inside the
//! domain object").

use serde::{Deserialize, Serialize};

use ironmaint_core::{ArtifactId, Digest};

/// Kind of artifact an [`ArtifactRef`] points at.
///
/// "Other" is the escape hatch for kinds the core vocabulary doesn't
/// enumerate; spec §25 deliberately leaves the variant set open.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    SourceArchive,
    BuildArtifact,
    BinaryPackage,
    SourcePackage,
    LogFile,
    Report,
    Signature,
    /// Adapter-specific kinds the core vocabulary doesn't enumerate.
    Other(String),
}

impl std::fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceArchive => f.write_str("source_archive"),
            Self::BuildArtifact => f.write_str("build_artifact"),
            Self::BinaryPackage => f.write_str("binary_package"),
            Self::SourcePackage => f.write_str("source_package"),
            Self::LogFile => f.write_str("log_file"),
            Self::Report => f.write_str("report"),
            Self::Signature => f.write_str("signature"),
            Self::Other(s) => write!(f, "other({s})"),
        }
    }
}

/// Reference to an artifact: identity, kind, integrity, plus optional
/// media type and size (PHASE-0A.md §25).
///
/// The artifact's bytes live outside this struct; only the metadata
/// needed to find them and verify their integrity is stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub digest: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

impl ArtifactRef {
    #[must_use]
    pub fn new(id: ArtifactId, kind: ArtifactKind, digest: Digest) -> Self {
        Self {
            id,
            kind,
            digest,
            media_type: None,
            size_bytes: None,
        }
    }

    #[must_use]
    pub fn with_media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }

    #[must_use]
    pub fn with_size_bytes(mut self, size: u64) -> Self {
        self.size_bytes = Some(size);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{DigestAlgorithm, GitHashAlgorithm, GitObjectId};

    fn digest() -> Digest {
        Digest::new(DigestAlgorithm::Sha256, "0".repeat(64)).unwrap()
    }

    fn oid() -> GitObjectId {
        GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap()
    }

    #[test]
    fn artifact_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ArtifactKind::BinaryPackage).unwrap(),
            r#""binary_package""#
        );
        assert_eq!(
            serde_json::to_string(&ArtifactKind::SourceArchive).unwrap(),
            r#""source_archive""#
        );
    }

    #[test]
    fn artifact_kind_other_carries_string() {
        let k = ArtifactKind::Other("dud-build-log".to_string());
        let json = serde_json::to_string(&k).unwrap();
        let parsed: ArtifactKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, k);
    }

    #[test]
    fn artifact_ref_builder_sets_optional_fields() {
        let r = ArtifactRef::new(ArtifactId::new(), ArtifactKind::BuildArtifact, digest())
            .with_media_type("application/octet-stream")
            .with_size_bytes(4096);
        assert_eq!(r.media_type.as_deref(), Some("application/octet-stream"));
        assert_eq!(r.size_bytes, Some(4096));
    }

    #[test]
    fn artifact_ref_omits_none_optionals() {
        let r = ArtifactRef::new(ArtifactId::new(), ArtifactKind::LogFile, digest());
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("media_type"));
        assert!(!json.contains("size_bytes"));
    }

    #[test]
    fn artifact_ref_round_trips() {
        let r = ArtifactRef::new(ArtifactId::new(), ArtifactKind::Signature, digest())
            .with_media_type("application/pgp-signature");
        let json = serde_json::to_string(&r).unwrap();
        let parsed: ArtifactRef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn artifact_kind_display_is_snake_case() {
        assert_eq!(ArtifactKind::SourceArchive.to_string(), "source_archive");
        assert_eq!(
            ArtifactKind::Other("custom".into()).to_string(),
            "other(custom)"
        );
    }

    // Touch `oid()` so the unused-import warning is silenced by using
    // the helper at least once.
    #[test]
    fn oid_helper_produces_valid_object_id() {
        let _ = oid();
    }
}
