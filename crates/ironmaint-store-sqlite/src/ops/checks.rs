//! `CheckStore` impl (PHASE-0B.md §44).
//!
//! Mirrors the gate-definition pattern: id-keyed primary row
//! plus a per-job index for `list_checks_for_job`. Idempotent
//! upserts via `ON CONFLICT(check_id) DO UPDATE` so callers
//! that re-`put` a freshly-minted `CheckDefinition` overwrite
//! cleanly without leaking rows.

use std::str::FromStr;

use sqlx::SqlitePool;

use ironmaint_core::{CheckId, JobId};
use ironmaint_store::{CheckDefinition, StoreError, StoreErrorKind};

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn put_check(
    pool: &SqlitePool,
    check: &CheckDefinition,
) -> Result<(), StoreError> {
    let payload = encode_json(check)?;
    sqlx::query(
        "INSERT INTO check_definitions (check_id, job_id, candidate, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(check_id) DO UPDATE SET
             job_id = excluded.job_id,
             candidate = excluded.candidate,
             payload_json = excluded.payload_json",
    )
    .bind(check.id.to_string())
    .bind(check.job_id.to_string())
    .bind(check.candidate.to_string())
    .bind("1")
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

pub(crate) async fn get_check(
    pool: &SqlitePool,
    id: CheckId,
) -> Result<CheckDefinition, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM check_definitions WHERE check_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("check {id}")));
    };
    map_json(payload, "CheckDefinition")
}

pub(crate) async fn list_checks_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<CheckId>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT check_id FROM check_definitions WHERE job_id = ?1 ORDER BY check_id",
    )
    .bind(job_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(s,)| {
            CheckId::from_str(&s).map_err(|e| {
                StoreError::new(StoreErrorKind::Corrupt, format!("bad check_id {s:?}: {e}"))
            })
        })
        .collect()
}
