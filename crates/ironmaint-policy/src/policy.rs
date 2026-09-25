//! Policy references and baselines (§§33, §36).
//!
//! [`PolicyReference`] is "where to look" for a given assertion:
//! which authority, which section, which document. [`PolicyBaseline`]
//! is "this is the complete set of authorities we trust for
//! distribution X" — the snapshot an adapter consumes when it
//! builds out [`Obligation`]s for a candidate (§36).

use serde::{Deserialize, Serialize};

use ironmaint_core::{AuthorityId, Digest, DistributionRef};

/// Pointer to a specific passage within an authority (§33).
///
/// `section`, `title`, `source`, and `content_digest` are all
/// optional — the minimum is just the [`AuthorityId`], which is
/// enough to assert "Debian Policy says so". The optional fields
/// exist so a recorded reference can be reproduced without going
/// back to the authority.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PolicyReference {
    pub authority: AuthorityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<Digest>,
}

const SECTION_MAX: usize = 256;
const TITLE_MAX: usize = 512;
const SOURCE_MAX: usize = 2048;

impl PolicyReference {
    /// Construct a reference from an authority id only.
    #[must_use]
    pub fn new(authority: AuthorityId) -> Self {
        Self {
            authority,
            section: None,
            title: None,
            source: None,
            content_digest: None,
        }
    }

    #[must_use]
    pub fn with_section(mut self, section: impl Into<String>) -> Self {
        self.section = Some(section.into());
        self
    }

    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the source URL string (validated as opaque; not parsed).
    ///
    /// # Errors
    /// Returns `Err(&'static str)` if `source` exceeds
    /// [`SOURCE_MAX`] bytes.
    pub fn with_source(mut self, source: impl Into<String>) -> Result<Self, &'static str> {
        let s: String = source.into();
        if s.len() > SOURCE_MAX {
            return Err("policy source exceeds 2048 bytes");
        }
        self.source = Some(s);
        Ok(self)
    }

    #[must_use]
    pub fn with_content_digest(mut self, digest: Digest) -> Self {
        self.content_digest = Some(digest);
        self
    }

    /// Validate field lengths against the policy maxima. Builders
    /// already enforce their own fields; this checks section / title
    /// lengths for callers that set fields directly.
    ///
    /// # Errors
    /// Returns `Err(&'static str)` if `section` or `title` exceeds
    /// their maxima.
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(s) = &self.section {
            if s.len() > SECTION_MAX {
                return Err("policy section exceeds 256 bytes");
            }
        }
        if let Some(t) = &self.title {
            if t.len() > TITLE_MAX {
                return Err("policy title exceeds 512 bytes");
            }
        }
        Ok(())
    }
}

/// A snapshot of the trusted authorities for one distribution (§36).
///
/// `authorities` is the list adapters consume when they produce
/// [`Obligation`]s; `distribution` identifies which distribution's
/// baseline this is. The baseline is itself produced by the adapter,
/// not by core — core just stores it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PolicyBaseline {
    pub distribution: DistributionRef,
    pub authorities: Vec<ironmaint_core::AuthorityId>,
}

impl PolicyBaseline {
    #[must_use]
    pub fn new(distribution: DistributionRef) -> Self {
        Self {
            distribution,
            authorities: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_authorities(mut self, authorities: Vec<ironmaint_core::AuthorityId>) -> Self {
        self.authorities = authorities;
        self
    }

    /// Push an authority id, returning `self` for chaining.
    #[must_use]
    pub fn add_authority(mut self, authority: ironmaint_core::AuthorityId) -> Self {
        self.authorities.push(authority);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::DigestAlgorithm;

    fn digest() -> Digest {
        Digest::new(DigestAlgorithm::Sha256, "0".repeat(64)).unwrap()
    }

    #[test]
    fn policy_reference_minimum() {
        let r = PolicyReference::new(AuthorityId::new());
        assert!(r.section.is_none());
        assert!(r.title.is_none());
        assert!(r.source.is_none());
        assert!(r.content_digest.is_none());
    }

    #[test]
    fn policy_reference_builder_chain() {
        let r = PolicyReference::new(AuthorityId::new())
            .with_section("3.1")
            .with_title("Binary packages")
            .with_source("https://example.invalid/policy#3.1")
            .unwrap()
            .with_content_digest(digest());
        assert_eq!(r.section.as_deref(), Some("3.1"));
        assert_eq!(r.title.as_deref(), Some("Binary packages"));
        assert_eq!(
            r.source.as_deref(),
            Some("https://example.invalid/policy#3.1")
        );
        assert!(r.content_digest.is_some());
    }

    #[test]
    fn policy_reference_source_rejects_oversize() {
        let long = "x".repeat(SOURCE_MAX + 1);
        let err = PolicyReference::new(AuthorityId::new())
            .with_source(long)
            .unwrap_err();
        assert!(err.contains("2048"));
    }

    #[test]
    fn policy_reference_round_trips() {
        let r = PolicyReference::new(AuthorityId::new()).with_section("3.1");
        let json = serde_json::to_string(&r).unwrap();
        let parsed: PolicyReference = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn policy_reference_omits_none_optionals() {
        let r = PolicyReference::new(AuthorityId::new());
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("section"));
        assert!(!json.contains("title"));
        assert!(!json.contains("source"));
        assert!(!json.contains("content_digest"));
    }

    #[test]
    fn policy_reference_validate_rejects_oversize_section() {
        let r = PolicyReference::new(AuthorityId::new()).with_section("x".repeat(SECTION_MAX + 1));
        assert!(r.validate().is_err());
    }

    #[test]
    fn policy_baseline_new_is_empty() {
        let b = PolicyBaseline::new(DistributionRef::new(
            ironmaint_core::DistributionFamily::new("debian").unwrap(),
            ironmaint_core::DistributionRelease::new("bookworm").unwrap(),
        ));
        assert!(b.authorities.is_empty());
    }

    #[test]
    fn policy_baseline_builder_accumulates_authorities() {
        let b = PolicyBaseline::new(DistributionRef::new(
            ironmaint_core::DistributionFamily::new("fedora").unwrap(),
            ironmaint_core::DistributionRelease::new("rawhide").unwrap(),
        ))
        .add_authority(AuthorityId::new())
        .add_authority(AuthorityId::new());
        assert_eq!(b.authorities.len(), 2);
    }

    #[test]
    fn policy_baseline_round_trips() {
        let b = PolicyBaseline::new(DistributionRef::new(
            ironmaint_core::DistributionFamily::new("debian").unwrap(),
            ironmaint_core::DistributionRelease::new("sid").unwrap(),
        ))
        .add_authority(AuthorityId::new());
        let json = serde_json::to_string(&b).unwrap();
        let parsed: PolicyBaseline = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, b);
    }
}
