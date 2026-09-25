//! Schema-only newtypes for foreign types we depend on but that
//! don't (yet) derive `schemars::JsonSchema` directly.
//!
//! Use these as `#[schemars(with = "...")]` overrides on the wire
//! field. They are NOT used at runtime — serde handles the wire
//! format directly on the underlying `time::OffsetDateTime`. They
//! exist purely to give schemars a local `JsonSchema` impl without
//! running into the orphan rule (we cannot impl a foreign trait
//! like `JsonSchema` for a foreign type like `OffsetDateTime`).
//!
//! `uuid::Uuid` and `url::Url` are handled by the `uuid1` and `url`
//! features on `schemars` itself, so no override is needed for them.

use schemars::JsonSchema;
use schemars::schema::{InstanceType, Schema, SchemaObject};

/// Newtype whose schema describes an RFC3339 timestamp string.
pub struct Rfc3339DateTime;

impl JsonSchema for Rfc3339DateTime {
    fn schema_name() -> String {
        "Rfc3339DateTime".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            format: Some("date-time".into()),
            ..Default::default()
        }
        .into()
    }
}
