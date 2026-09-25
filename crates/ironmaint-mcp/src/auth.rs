//! Bearer-token auth for the MCP server (PHASE-0B.md §51).
//!
//! The token is read from a file at startup; every tool call
//! must present `Authorization: Bearer <token>`. The validator
//! is constant-time and stateless — it does not query a
//! credential service.

use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing token file: {0}")]
    MissingFile(String),
    #[error("token file is empty")]
    EmptyToken,
    #[error("token rejected")]
    Rejected,
    #[error("io: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken(String);

impl AuthToken {
    /// Construct an `AuthToken` directly. Use this when wiring
    /// tests; production paths go through `from_file` /
    /// `from_env`.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Read the token from a file. The file should contain only
    /// the token (whitespace-trimmed).
    pub fn from_file(path: &Path) -> Result<Self, AuthError> {
        let bytes = std::fs::read(path).map_err(|e| AuthError::Io(e.to_string()))?;
        let text = std::str::from_utf8(&bytes).map_err(|e| AuthError::Io(format!("utf-8: {e}")))?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(AuthError::EmptyToken);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Read the token from an environment variable. Same
    /// validation as `from_file`.
    pub fn from_env(name: &str) -> Result<Self, AuthError> {
        let raw = std::env::var_os(name).ok_or_else(|| AuthError::MissingFile(name.to_string()))?;
        let text = raw.to_string_lossy().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(AuthError::EmptyToken);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct TokenValidator {
    expected: String,
}

impl TokenValidator {
    #[must_use]
    pub fn new(expected: AuthToken) -> Self {
        Self {
            expected: expected.0,
        }
    }

    /// Constant-time compare via length-then-byte-accumulator.
    /// Not as tight as `subtle::ConstantTimeEq` but adequate
    /// for the 0B baseline; real CT compare lands in 0B+.
    pub fn validate(&self, presented: &str) -> Result<(), AuthError> {
        let p = presented.as_bytes();
        let e = self.expected.as_bytes();
        if p.len() != e.len() {
            return Err(AuthError::Rejected);
        }
        let mut diff: u8 = 0;
        for (a, b) in p.iter().zip(e.iter()) {
            diff |= a ^ b;
        }
        if diff == 0 {
            Ok(())
        } else {
            Err(AuthError::Rejected)
        }
    }
}

// (private helper trait removed in favour of std::fs::read.)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_accepts_matching_token() {
        let v = TokenValidator::new(AuthToken("secret".to_string()));
        assert!(v.validate("secret").is_ok());
    }

    #[test]
    fn validator_rejects_different_token() {
        let v = TokenValidator::new(AuthToken("secret".to_string()));
        assert!(v.validate("other").is_err());
    }

    #[test]
    fn validator_rejects_prefix() {
        let v = TokenValidator::new(AuthToken("secret".to_string()));
        assert!(v.validate("secre").is_err());
    }
}
