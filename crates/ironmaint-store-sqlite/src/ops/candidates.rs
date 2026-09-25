//! `CandidateStore` impl.

use std::str::FromStr;

use sqlx::SqlitePool;

use ironmaint_core::{
    CandidateFingerprint, CandidateId, JobId, ReleaseCandidateId, SourceCandidate,
};
use ironmaint_policy::ReleaseCandidate;
use ironmaint_store::{StoreError, StoreErrorKind};

use super::{encode_json, map_json, map_sqlx_err};

pub(crate) async fn put_source(
    pool: &SqlitePool,
    cand: &SourceCandidate,
) -> Result<CandidateId, StoreError> {
    let id = cand.id();
    let fp = cand.fingerprint().clone();
    let job = cand.job_id();
    let package_json = encode_json(cand.package())?;
    let payload = encode_json(cand)?;

    sqlx::query(
        "INSERT INTO source_candidates (candidate_id, job_id, fingerprint, package_json, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(candidate_id) DO UPDATE SET
             job_id = excluded.job_id,
             fingerprint = excluded.fingerprint,
             package_json = excluded.package_json,
             payload_json = excluded.payload_json",
    )
    .bind(id.to_string())
    .bind(job.to_string())
    .bind(fp.to_string())
    .bind(package_json)
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(id)
}

pub(crate) async fn get_source(
    pool: &SqlitePool,
    id: CandidateId,
) -> Result<SourceCandidate, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM source_candidates WHERE candidate_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("source candidate {id}")));
    };
    map_json(payload, "SourceCandidate")
}

pub(crate) async fn put_release(
    pool: &SqlitePool,
    cand: &ReleaseCandidate,
) -> Result<ReleaseCandidateId, StoreError> {
    let id = cand.id;
    let payload = encode_json(cand)?;
    sqlx::query(
        "INSERT INTO release_candidates (candidate_id, job_id, fingerprint, payload_json)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(candidate_id) DO UPDATE SET
             job_id = excluded.job_id,
             fingerprint = excluded.fingerprint,
             payload_json = excluded.payload_json",
    )
    .bind(id.to_string())
    .bind(cand.job_id.to_string())
    .bind(cand.source.to_string())
    .bind(payload)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(id)
}

pub(crate) async fn get_release(
    pool: &SqlitePool,
    id: ReleaseCandidateId,
) -> Result<ReleaseCandidate, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload_json FROM release_candidates WHERE candidate_id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let Some((payload,)) = row else {
        return Err(StoreError::not_found(format!("release candidate {id}")));
    };
    map_json(payload, "ReleaseCandidate")
}

pub(crate) async fn list_source_for_job(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Vec<CandidateId>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT candidate_id FROM source_candidates WHERE job_id = ?1 ORDER BY candidate_id",
    )
    .bind(job_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_err)?;
    rows.into_iter()
        .map(|(s,)| {
            CandidateId::from_str(&s).map_err(|e| {
                StoreError::new(
                    StoreErrorKind::Corrupt,
                    format!("bad candidate_id {s:?}: {e}"),
                )
            })
        })
        .collect()
}

pub(crate) async fn active_source(
    pool: &SqlitePool,
    job_id: JobId,
) -> Result<Option<CandidateId>, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT candidate_id FROM active_source_candidates WHERE job_id = ?1")
            .bind(job_id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    match row {
        Some((s,)) => Ok(Some(CandidateId::from_str(&s).map_err(|e| {
            StoreError::new(
                StoreErrorKind::Corrupt,
                format!("bad candidate_id {s:?}: {e}"),
            )
        })?)),
        None => Ok(None),
    }
}

pub(crate) async fn set_active_source(
    pool: &SqlitePool,
    job_id: JobId,
    id: CandidateId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO active_source_candidates (job_id, candidate_id) VALUES (?1, ?2)
         ON CONFLICT(job_id) DO UPDATE SET candidate_id = excluded.candidate_id",
    )
    .bind(job_id.to_string())
    .bind(id.to_string())
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

pub(crate) async fn find_source_by_fp(
    pool: &SqlitePool,
    fingerprint: &CandidateFingerprint,
) -> Result<Option<CandidateId>, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT candidate_id FROM source_candidates WHERE fingerprint = ?1")
            .bind(fingerprint.to_string())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    match row {
        Some((s,)) => Ok(Some(CandidateId::from_str(&s).map_err(|e| {
            StoreError::new(
                StoreErrorKind::Corrupt,
                format!("bad candidate_id {s:?}: {e}"),
            )
        })?)),
        None => Ok(None),
    }
}
