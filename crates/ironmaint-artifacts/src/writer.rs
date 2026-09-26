//! Atomic-write writer for the artifact store.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::hash::{Sha256Hex, Sha256Writer};
use crate::path::ArtifactRoot;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("dest already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("artifact exceeds limit of {limit} bytes (observed {observed})")]
    TooLarge { limit: u64, observed: u64 },
}

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Stream `reader` (anything `tokio::io::AsyncRead + Unpin`)
/// into a temp file under `staging/`, compute SHA-256 over the
/// bytes, then rename the tempfile to the final sharded path
/// and fsync the parent directory.
///
/// `abort` is an [`AbortToken`]; the default implementation
/// never short-circuits. The temp filename is unique within the
/// staging directory (a counter plus a pid suffix), so two
/// concurrent writers cannot race to the same temp path.
///
/// `max_artifact_bytes` is the per-write byte cap enforced
/// immediately before each chunk is hashed and written. `None`
/// disables the check (tests only). When the cap would be
/// exceeded, the staging temp is removed and `WriteError::TooLarge`
/// is returned; the partial is never promoted to the final
/// sharded path. See PHASE-0B.md §15.
pub async fn write_from<R, B>(
    root: &ArtifactRoot,
    mut reader: R,
    abort: B,
    max_artifact_bytes: Option<u64>,
) -> Result<Sha256Hex, WriteError>
where
    R: tokio::io::AsyncRead + Unpin,
    B: AbortToken,
{
    fs::create_dir_all(root.staging()).await?;

    let seq = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let tmp_name = format!("artifact-{pid}-{seq}.partial");
    let tmp_path = root.staging().join(&tmp_name);

    let mut hasher = Sha256Writer::new();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .await?;

    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        if abort.is_aborted() {
            // Best-effort cleanup of the partial file. The
            // crash_safe test inspects the staging dir to confirm
            // the partial remains; this is exactly that case.
            return Err(WriteError::Io(std::io::Error::other("aborted")));
        }
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(WriteError::Io(e)),
        };
        if let Some(limit) = max_artifact_bytes {
            let next = total.checked_add(n as u64).ok_or(WriteError::TooLarge {
                limit,
                observed: u64::MAX,
            })?;
            if next > limit {
                // Reject before hashing/writing so the staging
                // temp never holds more than `limit` bytes.
                let _ = fs::remove_file(&tmp_path).await;
                return Err(WriteError::TooLarge {
                    limit,
                    observed: next,
                });
            }
            total = next;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).await?;
    }
    file.sync_all().await?;
    drop(file);

    let digest = hasher.finalize();
    let final_path = root.final_path_for(digest.as_str());

    if fs::try_exists(&final_path).await.unwrap_or(false) {
        // Content-addressed: existing same-digest file is
        // semantically identical. Drop our partial and return
        // the existing digest.
        let _ = fs::remove_file(&tmp_path).await;
        return Ok(digest);
    }
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    // Use std::fs::rename — both paths are on the same filesystem
    // (the staging dir is a child of the artifact root).
    std::fs::rename(&tmp_path, &final_path)?;
    if let Some(parent) = final_path.parent() {
        // `fsync` the parent directory so the rename itself is
        // durable; PHASE-0B.md §12 calls out the parent fsync as
        // a crash-safety requirement.
        if let Ok(f) = std::fs::File::open(parent) {
            let _ = f.sync_all();
        }
    }
    let _ = ensure_gone(&tmp_path);
    Ok(digest)
}

fn ensure_gone(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

/// Trait for an abort flag the writer checks between reads.
pub trait AbortToken {
    fn is_aborted(&self) -> bool;
}

/// The default production abort token — never aborts.
#[derive(Debug, Default, Clone, Copy)]
pub struct NeverAbort;

impl AbortToken for NeverAbort {
    fn is_aborted(&self) -> bool {
        false
    }
}

#[doc(hidden)]
pub mod test_token {
    use super::AbortToken;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub struct AtomicAbort(pub AtomicBool);

    impl Default for AtomicAbort {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AtomicAbort {
        pub fn new() -> Self {
            Self(AtomicBool::new(false))
        }
        pub fn abort(&self) {
            self.0.store(true, Ordering::Release);
        }
    }

    impl AbortToken for &AtomicAbort {
        fn is_aborted(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }
}
