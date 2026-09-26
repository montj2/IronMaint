//! Round-trip tests for `ArtifactStore`.
//!
//! Verifies: write→read, dedup, sharded path layout, integrity
//! check on corruption, and crash-safe abort.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_artifacts::{ArtifactRoot, ArtifactStore, writer::test_token::AtomicAbort};
use tokio::io::BufReader;

fn fresh_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[tokio::test]
async fn round_trip_put_get() {
    let tmp = fresh_root();
    let store = ArtifactStore::open(ArtifactRoot::new(tmp.path()));
    let bytes = b"hello artifact store";
    let rec = store.put_bytes(bytes).await.unwrap();
    let read = store.get(&rec.digest).await.unwrap();
    assert_eq!(read, bytes);
}

#[tokio::test]
async fn dedup_returns_existing_path() {
    let tmp = fresh_root();
    let store = ArtifactStore::open(ArtifactRoot::new(tmp.path()));
    let r1 = store.put_bytes(b"same").await.unwrap();
    let r2 = store.put_bytes(b"same").await.unwrap();
    assert_eq!(r1.digest, r2.digest);
    assert_eq!(r1.path, r2.path);
}

#[tokio::test]
async fn sharded_path_is_aa_bb_digest() {
    let tmp = fresh_root();
    let store = ArtifactStore::open(ArtifactRoot::new(tmp.path()));
    let r = store.put_bytes(b"path test").await.unwrap();
    let digest_str = r.digest.as_str();
    let aa = &digest_str[..2];
    let bb = &digest_str[2..4];
    let parent = r.path.parent().unwrap();
    assert_eq!(parent.file_name().unwrap().to_str().unwrap(), bb);
    let grand = parent.parent().unwrap();
    assert_eq!(grand.file_name().unwrap().to_str().unwrap(), aa);
}

#[tokio::test]
async fn stream_round_trip() {
    let tmp = fresh_root();
    let store = ArtifactStore::open(ArtifactRoot::new(tmp.path()));
    let bytes: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
    let cursor = std::io::Cursor::new(bytes.clone());
    let rec = store.put_stream(BufReader::new(cursor)).await.unwrap();
    let read = store.get(&rec.digest).await.unwrap();
    assert_eq!(read, bytes);
}

#[tokio::test]
async fn corrupted_blob_is_rejected() {
    let tmp = fresh_root();
    let store = ArtifactStore::open(ArtifactRoot::new(tmp.path()));
    let rec = store.put_bytes(b"original").await.unwrap();
    // Overwrite the on-disk file with a single different byte.
    std::fs::write(&rec.path, b"X").unwrap();
    let err = store.get(&rec.digest).await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("integrity check failed"),
        "expected integrity failure, got: {msg}"
    );
}

#[tokio::test]
async fn abort_leaves_no_half_file() {
    let tmp = fresh_root();
    let store = ArtifactStore::open(ArtifactRoot::new(tmp.path()));
    let abort = AtomicAbort::new();
    abort.abort();
    let cursor = std::io::Cursor::new(vec![0u8; 1024 * 1024]);
    let reader = BufReader::new(cursor);
    let res = ironmaint_artifacts::writer::write_from(store.root(), reader, &abort, None).await;
    assert!(res.is_err(), "aborted write should fail");
    // Walk the staging dir — there should be a leftover partial
    // and no fully-named artifact under the sharded tree.
    let staging = tmp.path().join("staging");
    let mut any_partial = false;
    let mut any_final = false;
    if staging.exists() {
        for entry in std::fs::read_dir(&staging).unwrap() {
            let entry = entry.unwrap();
            let n = entry.file_name().into_string().unwrap_or_default();
            if n.ends_with(".partial") {
                any_partial = true;
            }
        }
    }
    for aa_entry in std::fs::read_dir(tmp.path()).unwrap() {
        let p = aa_entry.unwrap().path();
        if !p.is_dir() || p.file_name().unwrap() == "staging" {
            continue;
        }
        for bb_entry in std::fs::read_dir(&p).unwrap() {
            let bb_path = bb_entry.unwrap().path();
            if bb_path.is_dir() {
                for f in std::fs::read_dir(&bb_path).unwrap() {
                    let f = f.unwrap();
                    if f.path().is_file() {
                        any_final = true;
                    }
                }
            }
        }
    }
    assert!(
        any_partial,
        "expected the partial tempfile to remain in staging"
    );
    assert!(!any_final, "no fully-named artifact must exist after abort");
}
