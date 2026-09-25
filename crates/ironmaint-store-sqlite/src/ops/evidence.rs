//! `EvidenceStore` impl.

use sqlx::SqlitePool;

use ironmaint_core::{CandidateFingerprint, EvidenceId, JobId};
use ironmaint_evidence::Evidence;
use ironmaint_store::StoreError;

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn put(
    pool: &SqlitePool,
    ev: &Evidence,
    job_id: JobId,
) -> Result<EvidenceId, StoreError> {
    let payload = encode_json(ev)?;
    sqlx::query(
        "INSERT INTO evidence (evidence_id, job_id, candidate, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(evidence_id) DO UPDATE SET
             job_id = excluded.job_id,
             candidate = excluded.candidate,
             payload_json = excluded.payload_json",
    )
    .bind(ev.id.to_string())
    .bind(job_id.to_string())
    .bind(ev.candidate.to_string())
    .bind("1")
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(ev.id)
}

pub(crate) async fn get(pool: &SqlitePool, id: EvidenceId) -> Result<Evidence, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM evidence WHERE evidence_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("evidence {id}")));
    };
    map_json(payload, "Evidence")
}

pub(crate) async fn list_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<Evidence>, StoreError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT payload_json FROM evidence WHERE job_id = ?1 ORDER BY evidence_id")
            .bind(job_id.to_string())
            .fetch_all(pool)
            .await
            .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(p,)| map_json(p, "Evidence"))
        .collect()
}

pub(crate) async fn list_for_candidate(
    pool: &SqlitePool,
    fingerprint: &CandidateFingerprint,
) -> Result<Vec<Evidence>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT payload_json FROM evidence WHERE candidate = ?1 ORDER BY evidence_id",
    )
    .bind(fingerprint.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(p,)| map_json(p, "Evidence"))
        .collect()
}
