//! `EventStore` impl.

use sqlx::SqlitePool;

use ironmaint_core::JobId;
use ironmaint_state::{JobEvent, StateTransitioned};
use ironmaint_store::{EventEnvelope, StoreError, StoreErrorKind};

use super::{encode_json, map_json, map_sqlx_err, rfc3339_string};

pub(crate) async fn append(pool: &SqlitePool, env: &EventEnvelope) -> Result<(), StoreError> {
    let mut tx = pool.begin().await.map_err(map_sqlx_err)?;
    let row: (Option<i64>,) = sqlx::query_as("SELECT MAX(sequence) FROM events WHERE job_id = ?1")
        .bind(env.job_id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;
    let current_max = row.0;
    let expected = current_max.unwrap_or(0) + 1;
    if env.sequence as i64 != expected {
        return Err(StoreError::sequence_out_of_range(
            env.job_id,
            expected as u64,
            env.sequence,
        ));
    }

    let (event_type, payload_json) = serialise_event(&env.event)?;

    sqlx::query(
        "INSERT INTO events (event_id, job_id, sequence, schema_version, occurred_at, event_type, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(env.event_id.to_string())
    .bind(env.job_id.to_string())
    .bind(env.sequence as i64)
    .bind("1")
    .bind(rfc3339_string(env.occurred_at)?)
    .bind(event_type)
    .bind(payload_json)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_err)?;

    tx.commit().await.map_err(map_sqlx_err)?;
    Ok(())
}

pub(crate) async fn get(
    pool: &SqlitePool,
    job_id: JobId,
    sequence: u64,
) -> Result<EventEnvelope, StoreError> {
    let row: Option<(String, i64, String, String, String, String)> = sqlx::query_as(
        "SELECT event_id, sequence, schema_version, occurred_at, event_type, payload_json
         FROM events WHERE job_id = ?1 AND sequence = ?2",
    )
    .bind(job_id.to_string())
    .bind(sequence as i64)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx_err)?;

    let Some((event_id, sequence, _schema, occurred_at, event_type, payload)) = row else {
        return Err(StoreError::not_found(format!(
            "event ({job_id}, {sequence})"
        )));
    };
    let event = deserialise_event(&event_type, &payload)?;
    Ok(EventEnvelope::new(
        parse_event_id(&event_id)?,
        job_id,
        sequence as u64,
        super::parse_rfc3339(&occurred_at)?,
        event,
    ))
}

pub(crate) async fn list(
    pool: &SqlitePool,
    job_id: JobId,
    start: u64,
    end: Option<u64>,
) -> Result<Vec<EventEnvelope>, StoreError> {
    let upper = end.map(|n| n as i64).unwrap_or(i64::MAX);
    let rows: Vec<(String, i64, String, String, String, String)> = sqlx::query_as(
        "SELECT event_id, sequence, schema_version, occurred_at, event_type, payload_json
         FROM events WHERE job_id = ?1 AND sequence BETWEEN ?2 AND ?3
         ORDER BY sequence ASC",
    )
    .bind(job_id.to_string())
    .bind(start as i64)
    .bind(upper)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx_err)?;

    rows.into_iter()
        .map(
            |(event_id, sequence, _schema, occurred_at, event_type, payload)| {
                let event = deserialise_event(&event_type, &payload)?;
                Ok(EventEnvelope::new(
                    parse_event_id(&event_id)?,
                    job_id,
                    sequence as u64,
                    super::parse_rfc3339(&occurred_at)?,
                    event,
                ))
            },
        )
        .collect()
}

pub(crate) async fn next_sequence(pool: &SqlitePool, job_id: JobId) -> Result<u64, StoreError> {
    let row: (Option<i64>,) = sqlx::query_as("SELECT MAX(sequence) FROM events WHERE job_id = ?1")
        .bind(job_id.to_string())
        .fetch_one(pool)
        .await
        .map_err(map_sqlx_err)?;
    Ok(row.0.unwrap_or(0).max(0) as u64 + 1)
}

fn serialise_event(event: &JobEvent) -> Result<(&'static str, String), StoreError> {
    match event {
        JobEvent::Transitioned(t) => {
            let payload = encode_json(t)?;
            Ok(("transitioned", payload))
        }
        JobEvent::Domain(id) => Ok(("domain", encode_json(id)?)),
    }
}

fn deserialise_event(event_type: &str, payload: &str) -> Result<JobEvent, StoreError> {
    match event_type {
        "transitioned" => {
            let t: StateTransitioned = map_json(payload.to_owned(), "StateTransitioned")?;
            Ok(JobEvent::Transitioned(t))
        }
        "domain" => {
            let id =
                map_json::<ironmaint_core::DomainEventId>(payload.to_owned(), "DomainEventId")?;
            Ok(JobEvent::Domain(id))
        }
        other => Err(StoreError::new(
            StoreErrorKind::Corrupt,
            format!("unknown event_type {other:?}"),
        )),
    }
}

fn parse_event_id(raw: &str) -> Result<ironmaint_core::MaintenanceEventId, StoreError> {
    use std::str::FromStr;
    ironmaint_core::MaintenanceEventId::from_str(raw).map_err(|e| {
        StoreError::new(
            StoreErrorKind::Corrupt,
            format!("bad event_id {raw:?}: {e}"),
        )
    })
}
