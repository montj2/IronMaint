//! Schema version marker (§60).
//!
//! A single `u16` wrapper that every wire envelope eventually embeds.
//! Internal value objects don't each carry a schema version; only
//! envelopes do, once they exist in 0A.3+.

use serde::{Deserialize, Serialize};

/// Wire schema version.
///
/// The first schema version is `1`. Increment when a breaking change
/// to the wire shape lands; consumers should reject envelopes with
/// newer versions they don't understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchemaVersion(pub u16);

impl SchemaVersion {
    /// Schema version 1 — the only version that exists in Phase 0A.
    pub const V1: Self = Self(1);

    /// The numeric value of the schema version.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl Default for SchemaVersion {
    fn default() -> Self {
        Self::V1
    }
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for SchemaVersion {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u16>().map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_constant_is_one() {
        assert_eq!(SchemaVersion::V1.value(), 1);
    }

    #[test]
    fn default_is_v1() {
        assert_eq!(SchemaVersion::default(), SchemaVersion::V1);
    }

    #[test]
    fn from_str_parses_decimal() {
        let v: SchemaVersion = "7".parse().unwrap();
        assert_eq!(v, SchemaVersion(7));
    }

    #[test]
    fn display_matches_inner() {
        assert_eq!(SchemaVersion(42).to_string(), "42");
    }
}
