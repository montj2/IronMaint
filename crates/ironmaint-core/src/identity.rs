//! Identifier newtypes (§6).
//!
//! Every domain object in IronMaint is keyed by a strongly-typed
//! identifier. This module defines all 15 ID newtypes via a single
//! macro: each wraps a `uuid::Uuid` and derives the same set of traits
//! so any ID is interchangeable in collections and serialization.
//!
//! All IDs use UUIDv7 when minted fresh (`ID::new()`); they can also
//! wrap an arbitrary existing UUID (`ID::from_uuid(...)`) for replay
//! and testing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Generate a UUIDv7-backed newtype identifier.
///
/// Every generated ID derives the full set of traits PHASE-0A.md §6
/// calls for (`Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash,
/// Serialize, Deserialize, Display, FromStr, Debug`), serializes as a
/// transparent UUID string, and exposes `new()` (UUIDv7),
/// `from_uuid(uuid::Uuid)` (opaque injection), and `as_uuid()`.
macro_rules! define_uuid_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
            JsonSchema,
        )]
        #[serde(transparent)]
        #[doc = concat!("Strongly-typed identifier (UUIDv7-backed).")]
        pub struct $name(uuid::Uuid);

        impl $name {
            #[doc = concat!("Mint a fresh `", stringify!($name), "` backed by a UUIDv7.")]
            #[must_use]
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7())
            }

            #[doc = concat!("Wrap an existing `uuid::Uuid` as a `", stringify!($name), "` (e.g. for replay).")]
            #[must_use]
            pub const fn from_uuid(id: uuid::Uuid) -> Self {
                Self(id)
            }

            /// Borrow the underlying `uuid::Uuid`.
            #[must_use]
            pub const fn as_uuid(&self) -> uuid::Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = CoreError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                uuid::Uuid::parse_str(s)
                    .map(Self)
                    .map_err(|e| CoreError::invalid_id(e.to_string()))
            }
        }
    };
}

define_uuid_id!(JobId);
define_uuid_id!(CandidateId);
define_uuid_id!(ReleaseCandidateId);
define_uuid_id!(IssueActionId);
define_uuid_id!(ApprovalId);
define_uuid_id!(OperationId);
define_uuid_id!(ArtifactId);
define_uuid_id!(MaintenanceEventId);
define_uuid_id!(DomainEventId);
define_uuid_id!(IssueProviderId);
define_uuid_id!(ActorId);
define_uuid_id!(AuthorityId);
define_uuid_id!(EvidenceId);
define_uuid_id!(GateId);
define_uuid_id!(ObligationId);
// Check identifier (Phase 0B §45). Identifies a specific check
// invocation within a candidate's BuildPlan / QaPlan. Allocated by
// the runtime when materialising an adapter's planned check into a
// durable, executable record. Phase 0A.4's `PlannedCheck` carries no
// ID; CheckId is introduced purely additively in 0B for the executor's
// bookkeeping.
define_uuid_id!(CheckId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_ids_are_distinct() {
        let a = JobId::new();
        let b = JobId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn display_round_trips_through_from_str() {
        let id = CandidateId::new();
        let printed = id.to_string();
        let parsed: CandidateId = printed.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn from_uuid_then_as_uuid_round_trip() {
        let raw = uuid::Uuid::now_v7();
        let id = AuthorityId::from_uuid(raw);
        assert_eq!(id.as_uuid(), raw);
    }

    #[test]
    fn default_is_a_fresh_id() {
        let a = ApprovalId::default();
        let b = ApprovalId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn from_str_rejects_garbage() {
        let err = "not-a-uuid".parse::<EvidenceId>().unwrap_err();
        assert_eq!(err.kind, crate::CoreErrorKind::InvalidId);
    }

    #[test]
    fn serialize_is_transparent_uuid_string() {
        let id = JobId::from_uuid(uuid::Uuid::nil());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"00000000-0000-0000-0000-000000000000\"");
    }

    #[test]
    fn deserialize_restores_same_id() {
        let id = GateId::from_uuid(uuid::Uuid::from_u128(
            0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
        ));
        let json = serde_json::to_string(&id).unwrap();
        let parsed: GateId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn ids_implement_ord_for_collections() {
        let mut ids = [ArtifactId::new(), ArtifactId::new(), ArtifactId::new()];
        ids.sort();
        // No assertion needed beyond "this compiles and sorts" — the derive
        // is what we're validating.
        assert_eq!(ids.len(), 3);
    }
}
