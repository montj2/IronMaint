//! `DaemonLock` — exclusive OS-level lock on the state directory.
//!
//! PHASE-0B.md §10: two daemons against the same state directory
//! must not silently corrupt each other's database. We use
//! `fs2::FileExt::try_lock_exclusive` so contention fails fast
//! instead of blocking the daemon startup path.

use std::fs::OpenOptions;
use std::path::Path;

use fs2::FileExt;

/// Holds the daemon lock file for the lifetime of the store.
///
/// The OS releases the flock when the file is closed (dropped);
/// we intentionally do not call `File::unlock` explicitly because
/// that API is post-MSRV (`std::fs::File::unlock` is stable from
/// Rust 1.89, and IronMaint's MSRV is 1.85.0).
#[derive(Debug)]
pub struct DaemonLock {
    /// Backing file. Kept open for the lock's lifetime; closing
    /// the file releases the OS-level flock.
    #[allow(dead_code)]
    file: std::fs::File,
}

impl DaemonLock {
    /// Acquire the lock file at `<state_dir>/ironmaint.lock`.
    /// Returns an error if another process holds it.
    pub(crate) fn acquire(state_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(state_dir)
            .map_err(|e| format!("create state dir {}: {e}", state_dir.display()))?;
        let lock_path = state_dir.join("ironmaint.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| format!("open lock file {}: {e}", lock_path.display()))?;
        file.try_lock_exclusive()
            .map_err(|_| "another ironmaintd is using this state directory".to_owned())?;
        Ok(Self { file })
    }

    /// Construct an empty placeholder lock for tests that build a
    /// `SqliteStore` against an in-memory SQLite and therefore
    /// have no real lock file to hold.
    #[doc(hidden)]
    pub fn empty_for_tests() -> Self {
        // Open `/dev/null` to satisfy `File`; the file's drop
        // releases the placeholder.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .expect("/dev/null must exist on test hosts");
        Self { file }
    }
}
