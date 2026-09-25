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

#[derive(Debug)]
pub struct ArtifactStore {
    root: ArtifactRoot,
}

impl ArtifactStore {
    pub fn open(root: ArtifactRoot) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &ArtifactRoot {
        &self.root
    }

    /// Write `bytes` to the store and return the content record.
    pub async fn put_bytes(&self, bytes: &[u8]) -> Result<Record, StoreError> {
        let digest = sha256_of_bytes(bytes);
        let path = self.root.final_path_for(digest.as_str());
        write_atomic(&self.root, &path, bytes).await?;
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
        let digest = writer::write_from(&self.root, reader, NeverAbort)
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
) -> Result<(), StoreError> {
    // For `put_bytes` we delegate to `write_from` over a `&[u8]`
    // by wrapping it in `Cursor`.
    use tokio::io::BufReader;
    let cursor = std::io::Cursor::new(bytes.to_vec());
    let reader = BufReader::new(cursor);
    let digest = writer::write_from(root, reader, NeverAbort)
        .await
        .map_err(StoreError::Write)?;
    // Sanity-check that the produced path lines up with the
    // expected one — write_from always returns the correct
    // path under the same root, so this is purely defensive.
    let produced = root.final_path_for(digest.as_str());
    debug_assert_eq!(&produced, final_path, "digest path mismatch");
    Ok(())
}
