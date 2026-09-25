//! `OperationStore` impl.

use std::str::FromStr;

use sqlx::SqlitePool;

use ironmaint_core::{JobId, OperationId};
use ironmaint_policy::{AuthorizationState, PrivilegedOperation};
use ironmaint_store::{StoreError, StoreErrorKind};

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn put(
    pool: &SqlitePool,
    op: &PrivilegedOperation,
    job_id: JobId,
) -> Result<OperationId, StoreError> {
    let payload = encode_json(op)?;
    sqlx::query(
        "INSERT INTO operations (operation_id, job_id, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(operation_id) DO UPDATE SET
             job_id = excluded.job_id,
             payload_json = excluded.payload_json",
    )
    .bind(op.id.to_string())
    .bind(job_id.to_string())
    .bind("1")
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(op.id)
}

pub(crate) async fn get(
    pool: &SqlitePool,
    id: OperationId,
) -> Result<PrivilegedOperation, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM operations WHERE operation_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("operation {id}")));
    };
    map_json(payload, "PrivilegedOperation")
}

pub(crate) async fn update(
    pool: &SqlitePool,
    id: OperationId,
    updated: &PrivilegedOperation,
) -> Result<(), StoreError> {
    let payload = encode_json(updated)?;
    let rows = sqlx::query("UPDATE operations SET payload_json = ?1 WHERE operation_id = ?2")
        .bind(payload)
        .bind(id.to_string())
        .execute(pool)
        .await
        .map_err(map_sqlx_err)?;
    if rows.rows_affected() == 0 {
        return Err(StoreError::not_found(format!("operation {id}")));
    }
    Ok(())
}

pub(crate) async fn list_executing(pool: &SqlitePool) -> Result<Vec<OperationId>, StoreError> {
    // Execute state lives inside the payload JSON; we read every
    // row and filter on `AuthorizationState::Executing`.
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT payload_json FROM operations ORDER BY operation_id")
            .fetch_all(pool)
            .await
            .map_err(map_sqlx_err)?;
    let mut out = Vec::new();
    for (payload,) in rows {
        let op: PrivilegedOperation = map_json(payload.clone(), "PrivilegedOperation")?;
        if matches!(op.authorization, AuthorizationState::Executing) {
            out.push(op.id);
        }
    }
    Ok(out)
}

pub(crate) async fn list_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<OperationId>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT operation_id FROM operations WHERE job_id = ?1 ORDER BY operation_id",
    )
    .bind(job_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(s,)| {
            OperationId::from_str(&s).map_err(|e| {
                StoreError::new(
                    StoreErrorKind::Corrupt,
                    format!("bad operation_id {s:?}: {e}"),
                )
            })
        })
        .collect()
}
