//! `SqliteStoreConfig` — opens a SQLite-backed store.

use std::path::PathBuf;

/// Configuration for [`crate::SqliteStore::open`].
#[derive(Clone, Debug)]
pub struct SqliteStoreConfig {
    /// Directory holding `ironmaint.sqlite` and the daemon lock.
    /// Created if absent.
    pub state_dir: PathBuf,
    /// PRAGMA `busy_timeout`, in milliseconds.
    ///
    /// Default: 5000. Higher values surface lock contention as a
    /// retry opportunity rather than a fast `SQLITE_BUSY`.
    pub busy_timeout_ms: u64,
    /// Maximum connections in the pool. 0 → sqlx default.
    pub max_connections: u32,
    /// Directory containing `NNNN_*.sql` migrations. `None`
    /// disables startup migration (used by tests that build a
    /// synthetic schema separately).
    pub migrations_dir: Option<PathBuf>,
}

impl SqliteStoreConfig {
    /// Convenience constructor for the common case: state dir
    /// given, all other fields at default.
    #[must_use]
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            busy_timeout_ms: 5_000,
            max_connections: 4,
            migrations_dir: None,
        }
    }

    #[must_use]
    pub fn with_migrations_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.migrations_dir = Some(dir.into());
        self
    }
}
