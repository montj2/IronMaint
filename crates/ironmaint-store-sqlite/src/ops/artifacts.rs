//! `ArtifactMetadataStore` impl.

use std::str::FromStr;

use sqlx::SqlitePool;

use ironmaint_core::{ArtifactId, JobId};
use ironmaint_store::{StoreError, StoreErrorKind, artifact::ArtifactRecord};

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn put(pool: &SqlitePool, rec: &ArtifactRecord) -> Result<ArtifactId, StoreError> {
    let payload = encode_json(rec)?;
    sqlx::query(
        "INSERT INTO artifact_metadata (artifact_id, job_id, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(artifact_id) DO UPDATE SET
             job_id = excluded.job_id,
             payload_json = excluded.payload_json",
    )
    .bind(rec.id.to_string())
    .bind(rec.job_id.to_string())
    .bind("1")
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(rec.id)
}

pub(crate) async fn get(pool: &SqlitePool, id: ArtifactId) -> Result<ArtifactRecord, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM artifact_metadata WHERE artifact_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("artifact {id}")));
    };
    map_json(payload, "ArtifactRecord")
}

pub(crate) async fn exists(pool: &SqlitePool, id: ArtifactId) -> Result<bool, StoreError> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM artifact_metadata WHERE artifact_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    Ok(row.is_some())
}

pub(crate) async fn list_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<ArtifactId>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT artifact_id FROM artifact_metadata WHERE job_id = ?1 ORDER BY artifact_id",
    )
    .bind(job_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(s,)| {
            ArtifactId::from_str(&s).map_err(|e| {
                StoreError::new(
                    StoreErrorKind::Corrupt,
                    format!("bad artifact_id {s:?}: {e}"),
                )
            })
        })
        .collect()
}
