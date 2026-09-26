//! Artifact store facade.

use std::path::PathBuf;

use time::OffsetDateTime;

use crate::hash::{Sha256Hex, sha256_of_bytes};
use crate::path::ArtifactRoot;
use crate::reader::{self, ReadError};
use crate::writer::{self, NeverAbort, WriteError};

/// Result of a successful write: the digest plus the on-disk path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub digest: Sha256Hex,
    pub path: PathBuf,
    pub size: u64,
    pub stored_at: OffsetDateTime,
}

/// Configurable knobs for [`ArtifactStore`].
///
/// Today only the per-write byte cap (PHASE-0B.md §15) is exposed.
/// Per-job caps and stdout/stderr caps belong on the executor
/// (PHASE-0B.md §15, deferred to 0B.4).
#[derive(Debug, Clone)]
pub struct ArtifactStoreConfig {
    /// Per-write byte cap. `None` disables enforcement (used in
    /// tests that want unbounded writes).
    ///
    /// Production default: `Some(256 * 1024 * 1024)` (256 MiB).
    pub max_artifact_bytes: Option<u64>,
}

impl Default for ArtifactStoreConfig {
    fn default() -> Self {
        Self {
            max_artifact_bytes: Some(256 * 1024 * 1024),
        }
    }
}

#[derive(Debug)]
pub struct ArtifactStore {
    root: ArtifactRoot,
    config: ArtifactStoreConfig,
}

impl ArtifactStore {
    /// Open with default config (256 MiB cap, see
    /// [`ArtifactStoreConfig::default`]).
    pub fn open(root: ArtifactRoot) -> Self {
        Self::open_with(root, ArtifactStoreConfig::default())
    }

    /// Open with explicit configuration. The configuration is
    /// read at open time and applied to all subsequent writes;
    /// it is not re-read during the store's lifetime.
    ///
    /// As a side effect this sweeps any orphan `.partial`
    /// tempfiles left behind in `staging/` by a crashed prior
    /// process (PHASE-0B.md §12, §90 crash-safety claim).
    pub fn open_with(root: ArtifactRoot, config: ArtifactStoreConfig) -> Self {
        sweep_staging(&root);
        Self { root, config }
    }

    /// Builder-style override of the per-write byte cap. `None`
    /// disables enforcement.
    #[must_use]
    pub fn with_max_artifact_bytes(mut self, limit: Option<u64>) -> Self {
        self.config.max_artifact_bytes = limit;
        self
    }

    /// The currently-configured per-write byte cap, or `None` if
    /// disabled.
    #[must_use]
    pub fn max_artifact_bytes(&self) -> Option<u64> {
        self.config.max_artifact_bytes
    }

    pub fn root(&self) -> &ArtifactRoot {
        &self.root
    }

    /// Write `bytes` to the store and return the content record.
    pub async fn put_bytes(&self, bytes: &[u8]) -> Result<Record, StoreError> {
        let digest = sha256_of_bytes(bytes);
        let path = self.root.final_path_for(digest.as_str());
        write_atomic(&self.root, &path, bytes, self.config.max_artifact_bytes).await?;
        Ok(Record {
            digest,
            path,
            size: bytes.len() as u64,
            stored_at: OffsetDateTime::now_utc(),
        })
    }

    /// Stream from `reader` into the store and return the record.
    pub async fn put_stream<R>(&self, reader: R) -> Result<Record, StoreError>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let digest = writer::write_from(
            &self.root,
            reader,
            NeverAbort,
            self.config.max_artifact_bytes,
        )
        .await
        .map_err(StoreError::Write)?;
        let path = self.root.final_path_for(digest.as_str());
        let stored_at = OffsetDateTime::now_utc();
        Ok(Record {
            digest,
            path,
            size: 0, // stream size unknown without pre-counting
            stored_at,
        })
    }

    pub async fn get(&self, digest: &Sha256Hex) -> Result<Vec<u8>, StoreError> {
        let path = self.root.final_path_for(digest.as_str());
        reader::read_all(&path, digest)
            .await
            .map_err(StoreError::Read)
    }

    pub fn path_for(&self, digest: &Sha256Hex) -> PathBuf {
        self.root.final_path_for(digest.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Write(WriteError),
    #[error(transparent)]
    Read(ReadError),
}

async fn write_atomic(
    root: &ArtifactRoot,
    final_path: &PathBuf,
    bytes: &[u8],
    max_artifact_bytes: Option<u64>,
) -> Result<(), StoreError> {
    // For `put_bytes` we delegate to `write_from` over a `&[u8]`
    // by wrapping it in `Cursor`.
    use tokio::io::BufReader;
    let cursor = std::io::Cursor::new(bytes.to_vec());
    let reader = BufReader::new(cursor);
    let digest = writer::write_from(root, reader, NeverAbort, max_artifact_bytes)
        .await
        .map_err(StoreError::Write)?;
    // Sanity-check that the produced path lines up with the
    // expected one — write_from always returns the correct
    // path under the same root, so this is purely defensive.
    let produced = root.final_path_for(digest.as_str());
    debug_assert_eq!(&produced, final_path, "digest path mismatch");
    Ok(())
}

/// Remove orphan `.partial` tempfiles from `staging/`. Called
/// from [`ArtifactStore::open_with`] to clean up after a
/// crashed prior process (PHASE-0B.md §12, §90 crash-safety
/// claim that `path.rs` has documented but never implemented).
///
/// Idempotent: a missing or already-clean staging dir is a
/// no-op. Synchronous: this is a startup hook that runs at
/// most once per process, so blocking the small initial scan
/// is acceptable; switching to async would require restructuring
/// `open_with`'s return type. Best-effort per file: an
/// unreadable entry is skipped, not surfaced as an error,
/// because the alternative (refuse to open) is worse than the
/// problem we're solving.
fn sweep_staging(root: &ArtifactRoot) {
    let staging = root.staging();
    let entries = match std::fs::read_dir(&staging) {
        Ok(entries) => entries,
        Err(_) => return, // dir doesn't exist = clean
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_partial = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".partial"));
        if path.is_file() && is_partial {
            let _ = std::fs::remove_file(&path);
        }
    }
}
