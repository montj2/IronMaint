//! BLAKE3 fingerprint fixed-vector integration tests.
//!
//! These tests pin the exact hex digest that the recipe in
//! `ironmaint_core::candidate::compute_fingerprint` produces for a
//! representative set of inputs. A regression that changes any of:
//!
//! - the namespace tag (`ironmaint-candidate-v1\0`),
//! - the field order (family, release, name, version, repo, commit, tree),
//! - the length-prefix scheme (4-byte LE `u32`),
//! - the hash function (BLAKE3-256),
//!
//! will fail at least one of these tests, which is exactly the early
//! warning we want before the fingerprint becomes load-bearing in 0A.3+.
//!
//! ## Authoring protocol (PHASE-0A.md §73)
//!
//! Vectors are baked in by:
//! 1. Run `cargo test -p ironmaint-core --test fingerprint_vectors
//!    -- --ignored --nocapture` once. The `print_known_vectors` test
//!    prints the actual fingerprint for each input combination below.
//! 2. Paste the printed hex into the `EXPECTED_*` constants here.
//! 3. Drop the `#[ignore]` on `print_known_vectors`. Future runs assert.
//! 4. Re-run without `--ignored`. The real assertions now compare
//!    against the baked-in hex, no longer calling the recipe under
//!    test.

// Integration tests are compiled as a separate crate and don't inherit
// `cfg_attr(test, allow(...))` from `lib.rs`. Allow `unwrap`/`expect`
// here for the same reason as the unit tests: the constructors under
// test are pre-validated by construction.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, PackageIdentity, PackageName, PackageRevision, PackageVersion,
    RepositoryRef, SourceCandidate, VcsKind,
};
use time::macros::datetime;
use url::Url;

fn fam(s: &str) -> DistributionFamily {
    DistributionFamily::new(s).unwrap()
}
fn rel(s: &str) -> DistributionRelease {
    DistributionRelease::new(s).unwrap()
}
fn name(s: &str) -> PackageName {
    PackageName::new(s).unwrap()
}
fn ver(s: &str) -> PackageVersion {
    PackageVersion::new(s).unwrap()
}
fn git_oid(hex: &str) -> GitObjectId {
    GitObjectId::new(GitHashAlgorithm::Sha1, hex).unwrap()
}
fn repo(url: &str) -> RepositoryRef {
    RepositoryRef::new(VcsKind::Git, Url::parse(url).unwrap()).unwrap()
}
fn pkg(dist: &str, release: &str, n: &str, v: &str) -> PackageRevision {
    PackageRevision::new(
        PackageIdentity::new(DistributionRef::new(fam(dist), rel(release)), name(n)),
        ver(v),
    )
}

// Fixed inputs reused by every vector. The Debian and Fedora fixtures
// come straight from PHASE-0A.md §69; the others exercise the per-field
// sensitivity of the recipe.

const DEBIAN_REPO: &str = "https://salsa.debian.org/foo/foo.git";
const FEDORA_REPO: &str = "https://src.fedoraproject.org/rpms/foo.git";

const COMMIT_A: &str = "1111111111111111111111111111111111111111";
const COMMIT_B: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TREE_A: &str = "2222222222222222222222222222222222222222";
const TREE_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

// --- Vector 1: Debian unstable / foo 1.9.0-1 (PHASE-0A.md §69 Debian fixture) ---
fn debian_unstable_foo() -> SourceCandidate {
    SourceCandidate::new(
        ironmaint_core::JobId::new(),
        pkg("debian", "unstable", "foo", "1.9.0-1"),
        repo(DEBIAN_REPO),
        git_oid(COMMIT_A),
        git_oid(TREE_A),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

// --- Vector 2: Fedora rawhide / foo 1.9.0-1.fc45 (PHASE-0A.md §69 Fedora fixture) ---
fn fedora_rawhide_foo() -> SourceCandidate {
    SourceCandidate::new(
        ironmaint_core::JobId::new(),
        pkg("fedora", "rawhide", "foo", "1.9.0-1.fc45"),
        repo(FEDORA_REPO),
        git_oid(COMMIT_A),
        git_oid(TREE_A),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

// --- Vector 3: same as #1, different commit ---
fn debian_unstable_foo_commit_b() -> SourceCandidate {
    SourceCandidate::new(
        ironmaint_core::JobId::new(),
        pkg("debian", "unstable", "foo", "1.9.0-1"),
        repo(DEBIAN_REPO),
        git_oid(COMMIT_B),
        git_oid(TREE_A),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

// --- Vector 4: same as #1, different tree ---
fn debian_unstable_foo_tree_b() -> SourceCandidate {
    SourceCandidate::new(
        ironmaint_core::JobId::new(),
        pkg("debian", "unstable", "foo", "1.9.0-1"),
        repo(DEBIAN_REPO),
        git_oid(COMMIT_A),
        git_oid(TREE_B),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

// --- Vector 5: same as #1, different release (`bookworm` vs `unstable`) ---
fn debian_bookworm_foo() -> SourceCandidate {
    SourceCandidate::new(
        ironmaint_core::JobId::new(),
        pkg("debian", "bookworm", "foo", "1.9.0-1"),
        repo(DEBIAN_REPO),
        git_oid(COMMIT_A),
        git_oid(TREE_A),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

// ---- Authoring helper: print the actual fingerprints of every vector. ----
//
// Run with:
//   cargo test -p ironmaint-core --test fingerprint_vectors \
//     -- --ignored --nocapture
//
// Copy the printed hex into the EXPECTED_* constants below, then drop
// the `#[ignore]` and the helper is no longer needed — but we keep it
// as a debug aid for future contributors who need to regenerate.
#[test]
#[ignore = "authoring helper — prints the actual fingerprints; not a real assertion"]
fn print_known_vectors() {
    eprintln!("--- fingerprint_vectors.rs: actual hex per input ---");
    for (label, c) in [
        ("debian_unstable_foo", debian_unstable_foo()),
        ("fedora_rawhide_foo", fedora_rawhide_foo()),
        (
            "debian_unstable_foo_commit_b",
            debian_unstable_foo_commit_b(),
        ),
        ("debian_unstable_foo_tree_b", debian_unstable_foo_tree_b()),
        ("debian_bookworm_foo", debian_bookworm_foo()),
    ] {
        eprintln!("{label}: {}", c.fingerprint());
    }
}

// ---- Fixed vectors ----
//
// BLAKE3-256 digests captured via the `print_known_vectors` authoring
// helper above. The recipe in
// `ironmaint_core::candidate::compute_fingerprint` produces these
// exact values; changing any input or any part of the recipe requires
// regenerating them via `cargo test ... -- --ignored --nocapture`.

const EXPECTED_DEBIAN_UNSTABLE_FOO: &str =
    "1cf8eec82dc242df4a47536fd9420ca89095ab94498ae1fccd8d29a289deccc4";
const EXPECTED_FEDORA_RAWHIDE_FOO: &str =
    "fd6353cc0a21b29409b91d4be5d2d7a3e0659be258bd2ca0213039a9ffd60bea";
const EXPECTED_DEBIAN_UNSTABLE_FOO_COMMIT_B: &str =
    "aec3f1b16b0b796bf34e31fe6f25c1dbd70792fa8a269b37e232c2e8e921f2bc";
const EXPECTED_DEBIAN_UNSTABLE_FOO_TREE_B: &str =
    "27d6d7cf380104fdc3528fce4878acd221968c689c0eca220439fb7f86064b25";
const EXPECTED_DEBIAN_BOOKWORM_FOO: &str =
    "f47068099869830175ab3b4aa788e8e81ffd9333307670cca3858f0e51a11f93";

fn assert_hex(expected_hex: &str, c: &SourceCandidate, label: &str) {
    let actual = c.fingerprint().as_str();
    assert_eq!(
        actual, expected_hex,
        "fingerprint regression for `{label}`\nexpected: {expected_hex}\nactual:   {actual}",
    );
    // The fingerprint must also round-trip through CandidateFingerprint::from_hex.
    let parsed = CandidateFingerprint::from_hex(actual).expect("self-produced hex is valid");
    assert_eq!(parsed, *c.fingerprint());
}

#[test]
fn vector_debian_unstable_foo() {
    assert_hex(
        EXPECTED_DEBIAN_UNSTABLE_FOO,
        &debian_unstable_foo(),
        "debian_unstable_foo",
    );
}

#[test]
fn vector_fedora_rawhide_foo() {
    assert_hex(
        EXPECTED_FEDORA_RAWHIDE_FOO,
        &fedora_rawhide_foo(),
        "fedora_rawhide_foo",
    );
}

#[test]
fn vector_debian_unstable_foo_commit_b() {
    assert_hex(
        EXPECTED_DEBIAN_UNSTABLE_FOO_COMMIT_B,
        &debian_unstable_foo_commit_b(),
        "debian_unstable_foo_commit_b",
    );
}

#[test]
fn vector_debian_unstable_foo_tree_b() {
    assert_hex(
        EXPECTED_DEBIAN_UNSTABLE_FOO_TREE_B,
        &debian_unstable_foo_tree_b(),
        "debian_unstable_foo_tree_b",
    );
}

#[test]
fn vector_debian_bookworm_foo() {
    assert_hex(
        EXPECTED_DEBIAN_BOOKWORM_FOO,
        &debian_bookworm_foo(),
        "debian_bookworm_foo",
    );
}

#[test]
fn vectors_are_all_distinct() {
    // Even without the baked-in hex, we can assert the obvious property
    // that five different inputs produce five different fingerprints.
    let fps = [
        debian_unstable_foo(),
        fedora_rawhide_foo(),
        debian_unstable_foo_commit_b(),
        debian_unstable_foo_tree_b(),
        debian_bookworm_foo(),
    ];
    let mut digests: Vec<&str> = fps.iter().map(|c| c.fingerprint().as_str()).collect();
    digests.sort_unstable();
    digests.dedup();
    assert_eq!(
        digests.len(),
        fps.len(),
        "expected every vector fingerprint to be unique",
    );
}

#[test]
fn fingerprint_is_stable_across_two_runs() {
    // Same recipe, same inputs → same hex. A regression guard against
    // accidental introduction of process-global state into the hasher.
    let a = debian_unstable_foo();
    let b = debian_unstable_foo();
    assert_eq!(a.fingerprint(), b.fingerprint());
}

#[test]
fn candidate_fingerprint_from_hex_rejects_short_input() {
    let err = CandidateFingerprint::from_hex("a".repeat(63)).unwrap_err();
    assert_eq!(err.kind, ironmaint_core::CoreErrorKind::InvalidDigest);
}

#[test]
fn candidate_fingerprint_from_hex_rejects_long_input() {
    let err = CandidateFingerprint::from_hex("a".repeat(65)).unwrap_err();
    assert_eq!(err.kind, ironmaint_core::CoreErrorKind::InvalidDigest);
}

#[test]
fn candidate_fingerprint_from_hex_rejects_non_hex() {
    let mut hex = "a".repeat(63);
    hex.push('z');
    let err = CandidateFingerprint::from_hex(hex).unwrap_err();
    assert_eq!(err.kind, ironmaint_core::CoreErrorKind::InvalidDigest);
}
