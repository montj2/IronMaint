//! `WorkspaceRevision` — optimistic-concurrency token.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Optimistic-concurrency counter for a workspace. Bumped by
/// `apply_patch` (and other mutating operations) and used as a
/// CAS token by the manager (PHASE-0B.md §32).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    Default,
)]
#[serde(transparent)]
pub struct WorkspaceRevision(pub u64);

impl WorkspaceRevision {
    pub const ZERO: Self = Self(0);

    pub fn new() -> Self {
        Self::ZERO
    }

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for WorkspaceRevision {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl fmt::Display for WorkspaceRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rev:{}", self.0)
    }
}
