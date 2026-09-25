//! `WorkspaceId` — UUIDv7 identifier for a per-job workspace.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// UUIDv7 identifier for a per-job workspace.
///
/// Mirrors the shape of the IDs defined by core's
/// `define_uuid_id!` macro but declared directly here so we
/// don't widen core's macro surface.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct WorkspaceId(Uuid);

impl WorkspaceId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Timestamp from the UUIDv7, used for tests and audit logs.
    pub fn timestamp(&self) -> OffsetDateTime {
        match self.0.get_timestamp() {
            Some(ts) => {
                let (sec, _nsec) = ts.to_unix();
                OffsetDateTime::from_unix_timestamp(sec as i64)
                    .unwrap_or(OffsetDateTime::UNIX_EPOCH)
            }
            None => OffsetDateTime::UNIX_EPOCH,
        }
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for WorkspaceId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}
