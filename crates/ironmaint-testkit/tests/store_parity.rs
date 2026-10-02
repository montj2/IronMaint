//! S9 driver — [`MockStore`] against the store-conformance script.
//!
//! Half of the pair. The other half is `store_parity_sqlite.rs`, and
//! the two files are the same test in the same shape against the two
//! backends. Split rather than combined so that a failure names the
//! backend: "the mock violates the contract" and "SQLite violates the
//! contract" are different bugs with different fixes, and a combined
//! test reports only that one of them is wrong.
//!
//! The script itself is `ironmaint_testkit::assert_store_conformance`.
//! This file is the harness, not the assertions — the assertions are in
//! the crate so that a new backend gets them by adding one `mod`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_store::mock::MockStore;
use ironmaint_testkit::assert_store_conformance;

/// A fresh `MockStore` has a clean slate, so the script's cases are
/// independent of each other and of every other test in the binary.
#[tokio::test]
async fn mock_store_satisfies_the_store_contract() {
    assert_store_conformance(&MockStore::new()).await;
}
