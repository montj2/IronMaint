//! Opaque tool capability key (§50).
//!
//! Format: `<adapter-namespace>.<role>.<tool>`, where every segment
//! is non-empty and ASCII-alphanumeric (with `-` and `_` allowed).
//! Examples: `debian.build.sbuild`, `fedora.qa.rpmlint`.
//!
//! Core sees capability keys as opaque strings. Validation is purely
//! shape; semantic matching between key and tool executor is the
//! responsibility of the executor layer (Phase 0B).

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{AdapterError, AdapterErrorKind};

const MAX_LEN: usize = 256;

/// A validated tool capability key (§50).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ToolCapabilityKey(String);

impl ToolCapabilityKey {
    /// Construct a [`ToolCapabilityKey`] with shape validation.
    ///
    /// # Errors
    /// Returns `Err(AdapterError { kind: InvalidConfiguration, .. })`
    /// if `key` is empty, exceeds [`MAX_LEN`] bytes, contains empty
    /// segments, contains uppercase characters in the namespace, or
    /// contains characters outside `[A-Za-z0-9_-]` within a segment.
    pub fn new(key: impl Into<String>) -> Result<Self, AdapterError> {
        let s = key.into();
        validate(&s)?;
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }

    /// First dotted segment — the adapter namespace. Always lowercase
    /// ASCII for valid keys.
    #[must_use]
    pub fn namespace(&self) -> &str {
        self.0.split('.').next().unwrap_or("")
    }
}

impl fmt::Display for ToolCapabilityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ToolCapabilityKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

fn validate(s: &str) -> Result<(), AdapterError> {
    if s.is_empty() {
        return Err(invalid("tool capability key must not be empty"));
    }
    if s.len() > MAX_LEN {
        return Err(invalid(&format!(
            "tool capability key length {} exceeds max {MAX_LEN}",
            s.len()
        )));
    }
    let mut iter = s.split('.');
    let first = iter.next().unwrap_or("");
    if first.is_empty() {
        return Err(invalid(
            "tool capability key requires a non-empty adapter namespace",
        ));
    }
    if first != first.to_ascii_lowercase() {
        return Err(invalid("tool capability key namespace must be lowercase"));
    }
    if !first
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid(
            "tool capability key namespace contains illegal characters",
        ));
    }
    for seg in iter {
        if seg.is_empty() {
            return Err(invalid("tool capability key contains empty segments"));
        }
        if !seg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(invalid(
                "tool capability key segment contains illegal characters",
            ));
        }
    }
    Ok(())
}

fn invalid(msg: &str) -> AdapterError {
    AdapterError {
        kind: AdapterErrorKind::InvalidConfiguration,
        message: msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_examples() {
        for k in [
            "debian.build.sbuild",
            "debian.qa.lintian",
            "debian.test.autopkgtest",
            "fedora.build.mock",
            "fedora.qa.rpmlint",
            "x.y.z",
            "ns-only.1",
        ] {
            assert!(ToolCapabilityKey::new(k).is_ok(), "expected `{k}` accepted");
        }
    }

    #[test]
    fn rejects_empty() {
        let e = ToolCapabilityKey::new("").unwrap_err();
        assert_eq!(e.kind, AdapterErrorKind::InvalidConfiguration);
        assert!(e.message.contains("empty"));
    }

    #[test]
    fn rejects_too_long() {
        let s = format!("{}.{}", "a".repeat(MAX_LEN - 1), "b");
        let e = ToolCapabilityKey::new(s).unwrap_err();
        assert_eq!(e.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn rejects_empty_namespace() {
        let e = ToolCapabilityKey::new(".build.sbuild").unwrap_err();
        assert_eq!(e.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn rejects_uppercase_namespace() {
        let e = ToolCapabilityKey::new("Debian.build.sbuild").unwrap_err();
        assert_eq!(e.kind, AdapterErrorKind::InvalidConfiguration);
        assert!(e.message.contains("lowercase"));
    }

    #[test]
    fn rejects_empty_segment() {
        let e = ToolCapabilityKey::new("debian..sbuild").unwrap_err();
        assert_eq!(e.kind, AdapterErrorKind::InvalidConfiguration);
        assert!(e.message.contains("empty"));
    }

    #[test]
    fn rejects_illegal_characters() {
        for k in [
            "debian.build.sbuild!",
            "debian.build.sbu!ld",
            "debian.build.sb$d",
        ] {
            let e = ToolCapabilityKey::new(k).unwrap_err();
            assert_eq!(e.kind, AdapterErrorKind::InvalidConfiguration);
        }
    }

    #[test]
    fn round_trips_through_serde() {
        let k = ToolCapabilityKey::new("debian.build.sbuild").unwrap();
        let json = serde_json::to_string(&k).unwrap();
        assert_eq!(json, r#""debian.build.sbuild""#);
        let parsed: ToolCapabilityKey = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, k);
    }

    #[test]
    fn namespace_accessor() {
        let k = ToolCapabilityKey::new("debian.build.sbuild").unwrap();
        assert_eq!(k.namespace(), "debian");
    }
}
