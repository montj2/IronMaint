//! SQLite-backed implementations of the nine `ironmaint-store`
//! sub-traits.
//!
//! Each submodule corresponds to one sub-trait. Payload fields
//! (`payload_json`) carry the serialised value of the in-memory
//! type; the `*_id` column carries the entity's primary key so
//! lookups by id do not require JSON parsing. Foreign keys
//! reference `projections(job_id)` so a row cannot outlive its
//! owning projection.

use time::OffsetDateTime;

use ironmaint_store::{StoreError, StoreErrorKind};

/// SQL column type used for RFC3339 timestamps.
pub(crate) fn rfc3339_string(dt: OffsetDateTime) -> Result<String, StoreError> {
    dt.format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("format rfc3339: {e}")))
}

pub(crate) fn parse_rfc3339(s: &str) -> Result<OffsetDateTime, StoreError> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("parse rfc3339 {s:?}: {e}")))
}

pub(crate) fn map_sqlx_err(e: sqlx::Error) -> StoreError {
    if let sqlx::Error::RowNotFound = e {
        return StoreError::new(StoreErrorKind::NotFound, "row not found");
    }
    StoreError::new(StoreErrorKind::Backend, format!("sqlx: {e}"))
}

pub(crate) fn map_json<T: serde::de::DeserializeOwned>(
    raw: String,
    ctx: &str,
) -> Result<T, StoreError> {
    serde_json::from_str(&raw)
        .map_err(|e| StoreError::new(StoreErrorKind::Corrupt, format!("decode {ctx}: {e}")))
}

pub(crate) fn encode_json<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value)
        .map_err(|e| StoreError::new(StoreErrorKind::Backend, format!("encode json: {e}")))
}

pub mod artifacts;
pub mod candidates;
pub mod events;
pub mod evidence;
pub mod gates;
pub mod obligations;
pub mod operations;
pub mod projections;
pub mod workspaces;
