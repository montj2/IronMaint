//! Retry class taxonomy (PHASE-0B.md §28).
//!
//! Three classes are fixed by the spec; nothing else is allowed.
//!
//! - `Safe`: running the tool again produces the same
//!   observable effect. May retry indefinitely on infrastructure
//!   failure; bounded by an operator-supplied budget.
//! - `Conditional`: running the tool again may produce a *new*
//!   observable side effect. May retry at most once, and only on
//!   clearly transient infrastructure errors (worker timeout,
//!   network blip). Otherwise the call is treated as `Failed`.
//! - `Never`: the tool may have produced an irreversible
//!   side effect on the world (BTS submit, upload, signing). Zero
//!   retries. The agent must explicitly re-issue.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetryClass {
    Safe,
    Conditional,
    Never,
}

impl RetryClass {
    /// Maximum number of retries permitted by this class.
    /// `None` means "bounded by external budget"; `Some(0)` means
    /// no retries; `Some(1)` means at most one retry.
    #[must_use]
    pub const fn max_retries(self) -> Option<u32> {
        match self {
            Self::Safe => None,
            Self::Conditional => Some(1),
            Self::Never => Some(0),
        }
    }

    /// Classify a tool capability key by namespace. Tools in the
    /// `synthetic.*` namespace are always `Safe` (the fixture
    /// is a controlled in-process subprocess). Distribution tools
    /// are `Conditional` by default. Signing/upload are
    /// `Never`. This is a default; tools can override.
    #[must_use]
    pub fn default_for(key: &str) -> Self {
        let namespace = key.split('.').next().unwrap_or("");
        let last = key.rsplit('.').next().unwrap_or("");
        match (namespace, last) {
            ("synthetic", _) => Self::Safe,
            (_, "upload" | "sign" | "publish" | "submit") => Self::Never,
            _ => Self::Conditional,
        }
    }
}

impl fmt::Display for RetryClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Safe => "safe",
            Self::Conditional => "conditional",
            Self::Never => "never",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_retries_matches_spec() {
        assert_eq!(RetryClass::Safe.max_retries(), None);
        assert_eq!(RetryClass::Conditional.max_retries(), Some(1));
        assert_eq!(RetryClass::Never.max_retries(), Some(0));
    }

    #[test]
    fn default_for_namespace() {
        assert_eq!(
            RetryClass::default_for("synthetic.build.validate"),
            RetryClass::Safe
        );
        assert_eq!(
            RetryClass::default_for("debian.build.sbuild"),
            RetryClass::Conditional
        );
        assert_eq!(
            RetryClass::default_for("fedora.publish.upload"),
            RetryClass::Never
        );
    }

    #[test]
    fn display_uses_spec_wire_strings() {
        assert_eq!(RetryClass::Safe.to_string(), "safe");
        assert_eq!(RetryClass::Conditional.to_string(), "conditional");
        assert_eq!(RetryClass::Never.to_string(), "never");
    }
}
