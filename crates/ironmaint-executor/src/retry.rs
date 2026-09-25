//! Retry class taxonomy (PHASE-0B.md §27).
//!
//! Three classes are fixed by the spec; nothing else is allowed.
//!
//! - `Idempotent`: running the tool again produces the same
//!   observable effect. May retry indefinitely on infrastructure
//!   failure; bounded by an operator-supplied budget.
//! - `SideEffecting`: running the tool again may produce a *new*
//!   observable side effect. May retry at most once, and only on
//!   clearly transient infrastructure errors (worker timeout,
//!   network blip). Otherwise the call is treated as `Failed`.
//! - `Destructive`: the tool may have produced an irreversible
//!   side effect on the world (BTS submit, upload, signing). Zero
//!   retries. The agent must explicitly re-issue.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetryClass {
    Idempotent,
    SideEffecting,
    Destructive,
}

impl RetryClass {
    /// Maximum number of retries permitted by this class.
    /// `None` means "bounded by external budget"; `Some(0)` means
    /// no retries; `Some(1)` means at most one retry.
    #[must_use]
    pub const fn max_retries(self) -> Option<u32> {
        match self {
            Self::Idempotent => None,
            Self::SideEffecting => Some(1),
            Self::Destructive => Some(0),
        }
    }

    /// Classify a tool capability key by namespace. Tools in the
    /// `synthetic.*` namespace are always `Idempotent` (the fixture
    /// is a controlled in-process subprocess). Distribution tools
    /// are `SideEffecting` by default. Signing/upload are
    /// `Destructive`. This is a default; tools can override.
    #[must_use]
    pub fn default_for(key: &str) -> Self {
        let namespace = key.split('.').next().unwrap_or("");
        let last = key.rsplit('.').next().unwrap_or("");
        match (namespace, last) {
            ("synthetic", _) => Self::Idempotent,
            (_, "upload" | "sign" | "publish" | "submit") => Self::Destructive,
            _ => Self::SideEffecting,
        }
    }
}

impl fmt::Display for RetryClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Idempotent => "idempotent",
            Self::SideEffecting => "side_effecting",
            Self::Destructive => "destructive",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_retries_matches_spec() {
        assert_eq!(RetryClass::Idempotent.max_retries(), None);
        assert_eq!(RetryClass::SideEffecting.max_retries(), Some(1));
        assert_eq!(RetryClass::Destructive.max_retries(), Some(0));
    }

    #[test]
    fn default_for_namespace() {
        assert_eq!(
            RetryClass::default_for("synthetic.build.validate"),
            RetryClass::Idempotent
        );
        assert_eq!(
            RetryClass::default_for("debian.build.sbuild"),
            RetryClass::SideEffecting
        );
        assert_eq!(
            RetryClass::default_for("fedora.publish.upload"),
            RetryClass::Destructive
        );
    }
}
