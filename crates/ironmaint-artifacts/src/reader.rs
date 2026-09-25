//! Reader for the artifact store.
//!
//! The reader verifies that the file's bytes hash to the digest
//! in its path. If the digest mismatches, the store is corrupt
//! and an `IntegrityFailed` error is returned.

use std::path::{Path, PathBuf};

use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::hash::{Sha256Hex, Sha256Writer};

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found at {0}")]
    NotFound(PathBuf),
    #[error("integrity check failed for {path}: expected {expected}, got {actual}")]
    IntegrityFailed {
        path: PathBuf,
        expected: Sha256Hex,
        actual: Sha256Hex,
    },
}

pub async fn read_all(path: &Path, expected: &Sha256Hex) -> Result<Vec<u8>, ReadError> {
    if !fs::try_exists(path).await.unwrap_or(false) {
        return Err(ReadError::NotFound(path.to_path_buf()));
    }
    let mut file = fs::File::open(path).await?;
    let mut hasher = Sha256Writer::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut out = Vec::new();
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.extend_from_slice(&buf[..n]);
    }
    let actual = hasher.finalize();
    if &actual != expected {
        return Err(ReadError::IntegrityFailed {
            path: path.to_path_buf(),
            expected: expected.clone(),
            actual,
        });
    }
    Ok(out)
}
