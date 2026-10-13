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
    ///
    /// The allow is **load-bearing, not suppression**, and D-05 was
    /// wrong to list this site as one to remove. Verified by
    /// promoting `dead_code` to `deny` and reading what rustc says:
    ///
    /// ```text
    /// error: field `file` is never read
    /// note: `DaemonLock` has a derived impl for the trait `Debug`,
    ///       but this is intentionally ignored during dead code analysis
    /// ```
    ///
    /// So it is reported, the derived `Debug` does not rescue it, and
    /// the field cannot be deleted: dropping it closes the file,
    /// which releases the OS lock, which is the entire mechanism. A
    /// field held for its `Drop` is a real pattern that the lint
    /// cannot see, and the correct response is a reasoned allow
    /// rather than a delete that silently stops locking.
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
        file.try_lock_exclusive().map_err(|e| {
            // Distinguish contention from every other I/O failure.
            // The old form discarded the `io::Error` entirely and
            // reported *all* of them as "another ironmaintd is
            // using this state directory" — so a read-only
            // filesystem or a permissions problem sent an operator
            // hunting for a second daemon that was never running.
            if e.kind() == std::io::ErrorKind::WouldBlock {
                format!(
                    "another ironmaintd is using this state directory ({})",
                    state_dir.display()
                )
            } else {
                format!("lock {}: {e}", lock_path.display())
            }
        })?;
        Ok(Self { file })
    }

    /// Construct an empty placeholder lock for tests that build a
    /// `SqliteStore` against an in-memory SQLite and therefore
    /// have no real lock file to hold.
    #[doc(hidden)]
    pub fn empty_for_tests() -> Result<Self, String> {
        // Open `/dev/null` to satisfy `File`; the file's drop
        // releases the placeholder. This cannot fail in any
        // environment IronMaint supports, but it is a filesystem
        // call in `src/` and the no-panic policy (PHASE-0A.md §85)
        // is not relaxed for "cannot really happen" — the caller
        // turns this into a `StoreError` like every other
        // filesystem failure in this crate.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(|e| format!("open /dev/null: {e}"))?;
        Ok(Self { file })
    }
}
