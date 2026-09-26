//! Tests for `ArtifactStore::put_bytes_truncating` (PHASE-0B.md §15).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_artifacts::{ArtifactRoot, ArtifactStore, TruncationInfo, hash::sha256_of_bytes};

fn temp_store() -> (tempfile::TempDir, ArtifactStore) {
    let dir = tempfile::tempdir().unwrap();
    let root = ArtifactRoot::new(dir.path());
    let s = ArtifactStore::open(root);
    (dir, s)
}

#[tokio::test]
async fn put_bytes_truncating_caps_at_limit() {
    let (_dir, store) = temp_store();
    let big = vec![b'A'; 1024 * 1024]; // 1 MiB
    let (record, info) = store.put_bytes_truncating(&big, 4096).await.unwrap();
    assert!(info.truncated);
    assert_eq!(info.limit_bytes, 4096);
    assert_eq!(info.observed_before_truncate, big.len() as u64);
    assert_eq!(info.stored_bytes, 4096);
    assert_eq!(record.size, 4096);
    // SHA-256 is over the truncated prefix only.
    let expected = sha256_of_bytes(&vec![b'A'; 4096]);
    assert_eq!(record.digest, expected);
}

#[tokio::test]
async fn put_bytes_truncating_passthrough_when_under_limit() {
    let (_dir, store) = temp_store();
    let small = vec![b'B'; 1024];
    let (record, info) = store.put_bytes_truncating(&small, 4096).await.unwrap();
    assert!(!info.truncated);
    assert_eq!(info.stored_bytes, 1024);
    assert_eq!(info.observed_before_truncate, 1024);
    assert_eq!(record.size, 1024);
    // SHA-256 matches the full input.
    assert_eq!(record.digest, sha256_of_bytes(&small));
}

#[tokio::test]
async fn put_bytes_truncating_at_exact_limit_is_not_truncated() {
    let (_dir, store) = temp_store();
    let exact = vec![b'C'; 4096];
    let (record, info) = store.put_bytes_truncating(&exact, 4096).await.unwrap();
    assert!(!info.truncated);
    assert_eq!(info.stored_bytes, 4096);
    assert_eq!(record.size, 4096);
}

#[tokio::test]
async fn put_bytes_truncating_empty_input_is_not_truncated() {
    let (_dir, store) = temp_store();
    let (record, info) = store.put_bytes_truncating(&[], 4096).await.unwrap();
    assert!(!info.truncated);
    assert_eq!(info.stored_bytes, 0);
    assert_eq!(record.size, 0);
}

#[tokio::test]
async fn put_bytes_truncating_digest_differs_when_input_differs() {
    let (_dir, store) = temp_store();
    let a = vec![b'A'; 10_000];
    let b = vec![b'B'; 10_000];
    let (rec_a, info_a) = store.put_bytes_truncating(&a, 4096).await.unwrap();
    let (rec_b, info_b) = store.put_bytes_truncating(&b, 4096).await.unwrap();
    assert!(info_a.truncated && info_b.truncated);
    assert_ne!(rec_a.digest, rec_b.digest);
}

#[tokio::test]
async fn existing_put_bytes_still_hard_fails_on_oversize() {
    // Regression: `put_bytes` semantics unchanged. When the configured
    // per-write cap is exceeded, the call returns `WriteError::TooLarge`
    // (no truncated-as-success path).
    let dir = tempfile::tempdir().unwrap();
    let root = ArtifactRoot::new(dir.path());
    let store = ArtifactStore::open(root).with_max_artifact_bytes(Some(64));
    let big = vec![0u8; 65];
    let err = store.put_bytes(&big).await.unwrap_err();
    let s = format!("{err}");
    assert!(
        s.contains("limit") || s.contains("TooLarge"),
        "expected TooLarge, got: {s}"
    );
}

#[tokio::test]
async fn truncation_info_is_comparable() {
    let info1 = TruncationInfo {
        limit_bytes: 100,
        observed_before_truncate: 200,
        stored_bytes: 100,
        truncated: true,
    };
    let info2 = info1.clone();
    assert_eq!(info1, info2);
}
