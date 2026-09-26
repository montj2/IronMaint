//! `WorkspaceMetadataStore` impl.

use sqlx::SqlitePool;

use ironmaint_core::JobId;
use ironmaint_store::{StoreError, StoreErrorKind, workspace::WorkspaceState};

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn get(pool: &SqlitePool, handle: &str) -> Result<WorkspaceState, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM workspace_metadata WHERE handle = ?1")
            .bind(handle)
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("workspace {handle}")));
    };
    map_json(payload, "WorkspaceState")
}

pub(crate) async fn put(
    pool: &SqlitePool,
    state: &WorkspaceState,
    expected_revision: u64,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await.map_err(map_sqlx_err)?;
    let current: Option<(i64,)> =
        sqlx::query_as("SELECT revision FROM workspace_metadata WHERE handle = ?1")
            .bind(&state.handle)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;

    match current {
        Some((r,)) if r as u64 != expected_revision => {
            return Err(StoreError::conflict(format!(
                "workspace {}: expected_revision={expected_revision}, found={r}",
                state.handle
            )));
        }
        None if expected_revision != 0 => {
            return Err(StoreError::conflict(format!(
                "workspace {}: first write requires expected_revision=0, got {expected_revision}",
                state.handle
            )));
        }
        _ => {}
    }

    let payload = encode_json(state)?;
    sqlx::query(
        "INSERT INTO workspace_metadata (handle, job_id, revision, schema_version, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(handle) DO UPDATE SET
             job_id = excluded.job_id,
             revision = excluded.revision,
             payload_json = excluded.payload_json",
    )
    .bind(&state.handle)
    .bind(state.job_id.to_string())
    .bind(state.revision as i64)
    .bind("0B.3")
    .bind(payload)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_err)?;

    tx.commit().await.map_err(map_sqlx_err)?;
    Ok(())
}

pub(crate) async fn list_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<String>, StoreError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT handle FROM workspace_metadata WHERE job_id = ?1 ORDER BY handle")
            .bind(job_id.to_string())
            .fetch_all(pool)
            .await
            .map_err(map_sqlx_err)?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

// Keep StoreErrorKind in scope for diagnostic helpers used by
// future commits.
#[allow(dead_code)]
fn _kind() -> StoreErrorKind {
    StoreErrorKind::Conflict
}
