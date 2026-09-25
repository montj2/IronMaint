//! `ObligationStore` impl.

use std::str::FromStr;

use sqlx::SqlitePool;

use ironmaint_core::{JobId, ObligationId};
use ironmaint_policy::Obligation;
use ironmaint_store::{StoreError, StoreErrorKind};

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn put(
    pool: &SqlitePool,
    ob: &Obligation,
    job_id: JobId,
) -> Result<ObligationId, StoreError> {
    let payload = encode_json(ob)?;
    sqlx::query(
        "INSERT INTO obligations (obligation_id, job_id, candidate, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(obligation_id) DO UPDATE SET
             job_id = excluded.job_id,
             candidate = excluded.candidate,
             payload_json = excluded.payload_json",
    )
    .bind(ob.id.to_string())
    .bind(job_id.to_string())
    .bind(ob.candidate.to_string())
    .bind("1")
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(ob.id)
}

pub(crate) async fn get(pool: &SqlitePool, id: ObligationId) -> Result<Obligation, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM obligations WHERE obligation_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("obligation {id}")));
    };
    map_json(payload, "Obligation")
}

pub(crate) async fn update(
    pool: &SqlitePool,
    id: ObligationId,
    updated: &Obligation,
) -> Result<(), StoreError> {
    let payload = encode_json(updated)?;
    let rows = sqlx::query(
        "UPDATE obligations SET payload_json = ?1, candidate = ?2
         WHERE obligation_id = ?3",
    )
    .bind(payload)
    .bind(updated.candidate.to_string())
    .bind(id.to_string())
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    if rows.rows_affected() == 0 {
        return Err(StoreError::not_found(format!("obligation {id}")));
    }
    Ok(())
}

pub(crate) async fn list_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<ObligationId>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT obligation_id FROM obligations WHERE job_id = ?1 ORDER BY obligation_id",
    )
    .bind(job_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(s,)| {
            ObligationId::from_str(&s).map_err(|e| {
                StoreError::new(
                    StoreErrorKind::Corrupt,
                    format!("bad obligation_id {s:?}: {e}"),
                )
            })
        })
        .collect()
}
