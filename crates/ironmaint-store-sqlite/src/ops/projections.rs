//! `ProjectionStore` impl.

use std::str::FromStr;

use sqlx::SqlitePool;

use ironmaint_core::{JobId, JobProjection};
use ironmaint_state::ProjectionApply;
use ironmaint_store::{StoreError, StoreErrorKind};

use super::events::list as list_events;
use super::{encode_json, map_sqlx_err, parse_rfc3339, rfc3339_string};

pub(crate) async fn get(pool: &SqlitePool, job_id: JobId) -> Result<JobProjection, StoreError> {
    let row: Option<(String, String, String, String, i64, String, String)> = sqlx::query_as(
        "SELECT package_json, initiating_event_id, state, active_candidate_id,
                    version, created_at, updated_at
             FROM projections WHERE job_id = ?1",
    )
    .bind(job_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx_err)?;

    let Some((
        package_json,
        initiating_event_id,
        state,
        active_candidate_id,
        version,
        created_at,
        updated_at,
    )) = row
    else {
        return Err(StoreError::not_found(format!("projection for {job_id}")));
    };

    let package: ironmaint_core::PackageIdentity = serde_json::from_str(&package_json)
        .map_err(|e| StoreError::new(StoreErrorKind::Corrupt, format!("decode package: {e}")))?;
    let state = parse_state(&state)?;
    let active_candidate = match active_candidate_id.as_str() {
        "" => None,
        s => Some(ironmaint_core::CandidateId::from_str(s).map_err(|e| {
            StoreError::new(
                StoreErrorKind::Corrupt,
                format!("bad candidate_id {s:?}: {e}"),
            )
        })?),
    };
    let initiating = ironmaint_core::MaintenanceEventId::from_str(&initiating_event_id)
        .map_err(|e| StoreError::new(StoreErrorKind::Corrupt, format!("bad event_id: {e}")))?;

    Ok(JobProjection {
        job: ironmaint_core::MaintenanceJob::new(
            job_id,
            package,
            initiating,
            parse_rfc3339(&created_at)?,
        ),
        state,
        active_candidate,
        version: version as u64,
        updated_at: parse_rfc3339(&updated_at)?,
    })
}

pub(crate) async fn put(
    pool: &SqlitePool,
    projection: &JobProjection,
    expected_version: u64,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await.map_err(map_sqlx_err)?;
    let current: Option<(i64,)> =
        sqlx::query_as("SELECT version FROM projections WHERE job_id = ?1")
            .bind(projection.job.id.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;

    match current {
        Some((v,)) if v as u64 != expected_version => {
            return Err(StoreError::conflict(format!(
                "expected_version={expected_version}, found={v}"
            )));
        }
        None if expected_version != 0 => {
            return Err(StoreError::conflict(format!(
                "first write requires expected_version=0, got {expected_version}"
            )));
        }
        _ => {}
    }

    let package_json = encode_json(&projection.job.package)?;
    sqlx::query(
        "INSERT INTO projections (job_id, package_json, initiating_event_id, state, active_candidate_id,
                                   version, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(job_id) DO UPDATE SET
             package_json = excluded.package_json,
             initiating_event_id = excluded.initiating_event_id,
             state = excluded.state,
             active_candidate_id = excluded.active_candidate_id,
             version = excluded.version,
             updated_at = excluded.updated_at",
    )
    .bind(projection.job.id.to_string())
    .bind(package_json)
    .bind(projection.job.initiating_event.to_string())
    .bind(state_label(projection.state))
    .bind(
        projection
            .active_candidate
            .map(|c| c.to_string())
            .unwrap_or_default(),
    )
    .bind(projection.version as i64)
    .bind(rfc3339_string(projection.job.created_at)?)
    .bind(rfc3339_string(projection.updated_at)?)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_err)?;

    tx.commit().await.map_err(map_sqlx_err)?;
    Ok(())
}

pub(crate) async fn rebuild(pool: &SqlitePool, job_id: JobId) -> Result<JobProjection, StoreError> {
    let events = list_events(pool, job_id, 1, None).await?;
    let Some(first) = events.first() else {
        return Err(StoreError::not_found(format!("no events for {job_id}")));
    };
    let initial = match &first.event {
        ironmaint_state::JobEvent::Transitioned(t) => t.projection_after.clone(),
        ironmaint_state::JobEvent::Domain(_) => {
            return Err(StoreError::corrupt(
                "first event is a Domain reference; no projection to seed rebuild",
            ));
        }
    };
    let mut proj = initial;
    for env in events.iter().skip(1) {
        proj = ProjectionApply::apply(&proj, &env.event, env.occurred_at);
    }
    Ok(proj)
}

fn parse_state(s: &str) -> Result<ironmaint_core::JobState, StoreError> {
    // JobState serialises via serde as its label.
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|e| StoreError::new(StoreErrorKind::Corrupt, format!("bad JobState {s:?}: {e}")))
}

fn state_label(s: ironmaint_core::JobState) -> String {
    // Use serde to format the label.
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .unwrap_or_default()
}
