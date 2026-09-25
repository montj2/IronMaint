//! `GateStore` impl.

use std::str::FromStr;

use sqlx::SqlitePool;

use ironmaint_core::{CandidateFingerprint, GateId, JobId};
use ironmaint_evidence::{GateDefinition, GateResult};
use ironmaint_store::{StoreError, StoreErrorKind};

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn put_definition(
    pool: &SqlitePool,
    gate: &GateDefinition,
    job_id: JobId,
) -> Result<GateId, StoreError> {
    let payload = encode_json(gate)?;
    sqlx::query(
        "INSERT INTO gate_definitions (gate_id, job_id, candidate, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(gate_id) DO UPDATE SET
             job_id = excluded.job_id,
             candidate = excluded.candidate,
             payload_json = excluded.payload_json",
    )
    .bind(gate.id.to_string())
    .bind(job_id.to_string())
    .bind(gate.candidate.to_string())
    .bind("1")
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(gate.id)
}

pub(crate) async fn get_definition(
    pool: &SqlitePool,
    id: GateId,
) -> Result<GateDefinition, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM gate_definitions WHERE gate_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("gate definition {id}")));
    };
    map_json(payload, "GateDefinition")
}

pub(crate) async fn put_result(
    pool: &SqlitePool,
    gate_id: GateId,
    fingerprint: &CandidateFingerprint,
    result: &GateResult,
) -> Result<(), StoreError> {
    let payload = encode_json(result)?;
    sqlx::query(
        "INSERT INTO gate_results (gate_id, candidate, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(gate_id, candidate) DO UPDATE SET
             payload_json = excluded.payload_json",
    )
    .bind(gate_id.to_string())
    .bind(fingerprint.to_string())
    .bind("1")
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

pub(crate) async fn get_result(
    pool: &SqlitePool,
    gate_id: GateId,
    fingerprint: &CandidateFingerprint,
) -> Result<GateResult, StoreError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT payload_json FROM gate_results WHERE gate_id = ?1 AND candidate = ?2",
    )
    .bind(gate_id.to_string())
    .bind(fingerprint.to_string())
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!(
            "gate result ({gate_id}, {fingerprint})"
        )));
    };
    map_json(payload, "GateResult")
}

pub(crate) async fn list_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<GateId>, StoreError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT gate_id FROM gate_definitions WHERE job_id = ?1 ORDER BY gate_id")
            .bind(job_id.to_string())
            .fetch_all(pool)
            .await
            .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(s,)| {
            GateId::from_str(&s).map_err(|e| {
                StoreError::new(StoreErrorKind::Corrupt, format!("bad gate_id {s:?}: {e}"))
            })
        })
        .collect()
}
