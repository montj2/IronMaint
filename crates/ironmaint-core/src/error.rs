//! Core error type.
//!
//! Used by every other module in `ironmaint-core`. `thiserror`-based with
//! cause-preserving `From` impls; no `anyhow` anywhere in core
//! (PHASE-0A.md §83, §85).

use std::fmt;

use thiserror::Error;

/// Categorical kind of a [`CoreError`].
///
/// `Display` deliberately emits a short, lowercase, identifier-shaped
/// phrase — never a sentence — so error chains stay readable when
/// several errors are formatted together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreErrorKind {
    InvalidId,
    InvalidName,
    InvalidVersion,
    InvalidDigest,
    InvalidUrl,
    InvalidPath,
    InvalidTimestamp,
    InvalidEventSource,
}

impl fmt::Display for CoreErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::InvalidId => "invalid id",
            Self::InvalidName => "invalid name",
            Self::InvalidVersion => "invalid version",
            Self::InvalidDigest => "invalid digest",
            Self::InvalidUrl => "invalid url",
            Self::InvalidPath => "invalid path",
            Self::InvalidTimestamp => "invalid timestamp",
            Self::InvalidEventSource => "invalid event source",
        };
        f.write_str(s)
    }
}

/// Single error type for every domain constructor in `ironmaint-core`.
///
/// Most validation functions return `Result<T, CoreError>`. The `kind`
/// discriminates the failure category; `message` carries the human-
/// readable detail (and, for `From` conversions, the underlying cause
/// already stringified — we deliberately preserve it because losing
/// cause strings is forbidden by PHASE-0A.md §85's error-handling
/// policy).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{kind}: {message}")]
pub struct CoreError {
    pub kind: CoreErrorKind,
    pub message: String,
}

impl CoreError {
    /// Construct a `CoreError` from a kind and a message.
    pub fn new(kind: CoreErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Convenience constructor for `InvalidId`.
    pub fn invalid_id(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidId, message)
    }

    /// Convenience constructor for `InvalidName`.
    pub fn invalid_name(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidName, message)
    }

    /// Convenience constructor for `InvalidVersion`.
    pub fn invalid_version(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidVersion, message)
    }

    /// Convenience constructor for `InvalidDigest`.
    pub fn invalid_digest(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidDigest, message)
    }

    /// Convenience constructor for `InvalidUrl`.
    pub fn invalid_url(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidUrl, message)
    }

    /// Convenience constructor for `InvalidPath`.
    pub fn invalid_path(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidPath, message)
    }

    /// Convenience constructor for `InvalidTimestamp`.
    pub fn invalid_timestamp(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidTimestamp, message)
    }

    /// Convenience constructor for `InvalidEventSource`.
    pub fn invalid_event_source(message: impl Into<String>) -> Self {
        Self::new(CoreErrorKind::InvalidEventSource, message)
    }
}

impl From<uuid::Error> for CoreError {
    fn from(err: uuid::Error) -> Self {
        Self::invalid_id(err.to_string())
    }
}

impl From<url::ParseError> for CoreError {
    fn from(err: url::ParseError) -> Self {
        Self::invalid_url(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_formats_kind_then_message() {
        let err = CoreError::invalid_name("empty");
        assert_eq!(err.to_string(), "invalid name: empty");
    }

    #[test]
    fn from_uuid_error_preserves_cause() {
        let bad = uuid::Uuid::parse_str("not-a-uuid").unwrap_err();
        let err: CoreError = bad.into();
        assert_eq!(err.kind, CoreErrorKind::InvalidId);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn from_url_error_preserves_cause() {
        let bad = url::Url::parse("not a url").unwrap_err();
        let err: CoreError = bad.into();
        assert_eq!(err.kind, CoreErrorKind::InvalidUrl);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn kind_display_is_lowercase_phrase() {
        assert_eq!(CoreErrorKind::InvalidId.to_string(), "invalid id");
        assert_eq!(CoreErrorKind::InvalidPath.to_string(), "invalid path");
        assert_eq!(
            CoreErrorKind::InvalidEventSource.to_string(),
            "invalid event source"
        );
    }
}
