//! Negative tests for `apply_migrations` refusal logic (audit gap D).
//!
//! PHASE-0B.md §30 requires the daemon to refuse to run against a
//! downgraded or drifted migration history. The trait surface does
//! not expose `apply_migrations` directly, so these tests exercise
//! it via `SqliteStore::open` (the production entry point that
//! internally calls `apply_migrations`) plus a parallel raw-SQL
//! connection to poison `schema_migrations`.

use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Open a parallel raw-SQL pool against the same SQLite file the
/// `SqliteStore` writes to. Used to inject ghost rows into
/// `schema_migrations` between two `SqliteStore::open` calls.
async fn raw_pool(state_dir: &std::path::Path) -> sqlx::SqlitePool {
    let db_path = state_dir.join("ironmaint.sqlite");
    let opts = SqliteConnectOptions::new().filename(&db_path);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap()
}

fn config_with_workdir(workdir: &std::path::Path) -> SqliteStoreConfig {
    SqliteStoreConfig::new(workdir).with_migrations_dir(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations"),
    )
}

/// Apply migrations once successfully, then poison the recorded
/// history with a synthetic `v=2 / 0002_ghost.sql` row, then
/// reopen and assert the second open returns `StoreError::Corrupt`.
#[tokio::test]
async fn apply_migrations_refuses_downgraded_history() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    // First open: succeeds, records 0001_initial.sql.
    let store = SqliteStore::open(config_with_workdir(&state_dir))
        .await
        .unwrap();
    drop(store);

    // Inject a ghost row: recorded v=2 with no file on disk. The
    // disk max is 1, so recorded max=2 > disk max=1 must trigger
    // the downgrade refusal on reopen.
    let pool = raw_pool(&state_dir).await;
    sqlx::query(
        "INSERT INTO schema_migrations (version, filename, applied_at) \
         VALUES (2, '0002_ghost.sql', '2026-01-01 00:00:00+00:00')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    // Second open must fail.
    let err = SqliteStore::open(config_with_workdir(&state_dir))
        .await
        .unwrap_err();
    let kind = err.kind();
    assert!(
        matches!(kind, ironmaint_store::StoreErrorKind::Corrupt),
        "expected Corrupt, got {kind:?}: {err}"
    );
    let msg = format!("{err}");
    assert!(
        msg.contains("downgraded") || msg.contains("0002_ghost.sql"),
        "expected downgrade message, got: {msg}"
    );
}

/// Apply migrations once successfully, then rewrite the recorded
/// filename of v=1 to a name that no longer exists on disk, then
/// reopen and assert the second open returns `StoreError::Corrupt`
/// with a "missing from disk" message.
#[tokio::test]
async fn apply_migrations_refuses_drifted_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let store = SqliteStore::open(config_with_workdir(&state_dir))
        .await
        .unwrap();
    drop(store);

    // Rewrite the recorded filename to something that does not
    // exist on disk. The version stays at 1, so the downgrade
    // check is not triggered; the drift check is.
    let pool = raw_pool(&state_dir).await;
    sqlx::query(
        "UPDATE schema_migrations SET filename = '0001_renamed.sql' \
         WHERE version = 1",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let err = SqliteStore::open(config_with_workdir(&state_dir))
        .await
        .unwrap_err();
    let kind = err.kind();
    assert!(
        matches!(kind, ironmaint_store::StoreErrorKind::Corrupt),
        "expected Corrupt, got {kind:?}: {err}"
    );
    let msg = format!("{err}");
    assert!(
        msg.contains("missing from disk") || msg.contains("0001_renamed.sql"),
        "expected drift message, got: {msg}"
    );
}

/// Sanity: a clean open + close + reopen still succeeds (the new
/// refusal checks must not break the happy path).
#[tokio::test]
async fn apply_migrations_happy_path_still_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let store1 = SqliteStore::open(config_with_workdir(&state_dir))
        .await
        .unwrap();
    drop(store1);

    // Recorded set should be exactly the on-disk set after first open.
    let pool = raw_pool(&state_dir).await;
    let recorded_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;

    let disk_count = std::fs::read_dir(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations"),
    )
    .unwrap()
    .filter(|e| {
        e.as_ref()
            .unwrap()
            .path()
            .extension()
            .and_then(|s| s.to_str())
            == Some("sql")
    })
    .count() as i64;
    assert_eq!(
        recorded_count, disk_count,
        "every on-disk .sql must be recorded exactly once"
    );

    // Reopen must still succeed.
    let _store2 = SqliteStore::open(config_with_workdir(&state_dir))
        .await
        .unwrap();
}
