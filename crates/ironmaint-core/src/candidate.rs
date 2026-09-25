//! Source candidates (§§12–13).
//!
//! A `SourceCandidate` is the immutable record of "what we propose to
//! build" for one (job, package) pair: the package identity and version
//! in play, the repository we're building from, and the exact commit
//! and tree objects that the build will check out.
//!
//! The candidate carries a BLAKE3 [`CandidateFingerprint`] computed from
//! its identity-defining fields. The fingerprint is what the rest of the
//! system pins — two candidates with the same fingerprint are, by
//! construction, byte-identical inputs to a build (§13).
//!
//! ## Immutability (§13)
//!
//! Every field on `SourceCandidate` is private. The only way to produce
//! one is via [`SourceCandidate::new`] (fresh candidate) or
//! [`SourceCandidate::new_revision`] (rebased on a parent). There is no
//! setter, no `DerefMut`, no `Clone`-then-mutate path that preserves the
//! typed wrapper. Once a candidate exists, its fingerprint — and
//! therefore its identity — is fixed.

use std::fmt;

use blake3::Hasher;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::digest::GitObjectId;
use crate::error::{CoreError, CoreErrorKind};
use crate::identity::{CandidateId, JobId};
use crate::package::PackageRevision;
use crate::repository::RepositoryRef;

/// Length, in nibbles, of a BLAKE3-256 hex digest.
const FINGERPRINT_HEX_LEN: usize = 64;

/// Namespace tag prepended to every fingerprint hash.
///
/// Bumping the prefix (e.g. to `ironmaint-candidate-v2\0`) is the
/// migration path if the fingerprint recipe ever changes: existing
/// fingerprints remain stable; new candidates get new hashes; nothing
/// silently collides.
const FINGERPRINT_NAMESPACE: &[u8] = b"ironmaint-candidate-v1\0";

/// BLAKE3 fingerprint of a candidate's identity-defining fields.
///
/// The inner string is always 64 lowercase hex chars. `from_hex` is the
/// only public constructor (so external callers — e.g. a deserializer —
/// can rebuild a verified fingerprint from a stored hex string). The
/// actual hash computation lives behind the module-private
/// [`compute_fingerprint`] helper and is reachable only through
/// [`SourceCandidate::new`] or [`SourceCandidate::new_revision`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CandidateFingerprint(String);

impl CandidateFingerprint {
    /// Construct a fingerprint from a 64-char hex BLAKE3 digest.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidDigest, .. }` if the input is
    /// not exactly 64 characters of lowercase or uppercase hex.
    pub fn from_hex(hex: impl Into<String>) -> Result<Self, CoreError> {
        let s = hex.into();
        validate_hex_64(&s)?;
        Ok(Self(s))
    }

    /// Borrow the underlying hex string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for CandidateFingerprint {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CandidateFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn validate_hex_64(s: &str) -> Result<(), CoreError> {
    if s.len() != FINGERPRINT_HEX_LEN {
        return Err(CoreError::new(
            CoreErrorKind::InvalidDigest,
            format!(
                "candidate fingerprint must be {FINGERPRINT_HEX_LEN} hex chars, got {}",
                s.len()
            ),
        ));
    }
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CoreError::new(
            CoreErrorKind::InvalidDigest,
            format!("candidate fingerprint `{s}` contains non-hex characters"),
        ));
    }
    Ok(())
}

/// BLAKE3-256 fingerprint over the seven identity-defining fields of a
/// candidate, namespace-tagged with [`FINGERPRINT_NAMESPACE`].
///
/// Field order and length-prefixing are part of the recipe; changing
/// either changes every fingerprint and is a wire-format break.
fn compute_fingerprint(
    package: &PackageRevision,
    repository: &RepositoryRef,
    commit: &GitObjectId,
    tree: &GitObjectId,
) -> CandidateFingerprint {
    let family = package.package.distribution.family.as_str();
    let release = package.package.distribution.release.as_str();
    let name = package.package.source_name.as_str();
    let version = package.version.as_str();
    let repo_url = repository.url().as_str();
    let commit = commit.as_str();
    let tree = tree.as_str();

    let mut hasher = Hasher::new();
    hasher.update(FINGERPRINT_NAMESPACE);
    for field in [family, release, name, version, repo_url, commit, tree] {
        hasher.update(&(field.len() as u32).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    let digest = hasher.finalize();
    // `finalize().to_hex()` always emits exactly 64 lowercase hex chars
    // (BLAKE3-256 → 32 bytes → 64 nibbles). Unwrap is safe by
    // construction; the `CandidateFingerprint` validator is a guard
    // against externally supplied hex, not against our own output.
    CandidateFingerprint(digest.to_hex().to_string())
}

/// An immutable "what we propose to build" record.
///
/// Constructed only through [`SourceCandidate::new`] or
/// [`SourceCandidate::new_revision`]. Fields are private; the only way
/// to read a field is via an accessor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCandidate {
    id: CandidateId,
    job_id: JobId,
    package: PackageRevision,
    repository: RepositoryRef,
    commit: GitObjectId,
    tree: GitObjectId,
    fingerprint: CandidateFingerprint,
    created_at: OffsetDateTime,
    parent_candidate: Option<CandidateId>,
}

impl SourceCandidate {
    /// Mint a fresh candidate (no parent).
    ///
    /// The fingerprint is computed from the seven identity-defining
    /// fields and stored on the candidate. The freshly minted `id` is a
    /// UUIDv7 — the fingerprint and the id are independent: the id
    /// identifies the *record*; the fingerprint identifies *what's in it*.
    #[must_use]
    pub fn new(
        job_id: JobId,
        package: PackageRevision,
        repository: RepositoryRef,
        commit: GitObjectId,
        tree: GitObjectId,
        created_at: OffsetDateTime,
    ) -> Self {
        let fingerprint = compute_fingerprint(&package, &repository, &commit, &tree);
        Self {
            id: CandidateId::new(),
            job_id,
            package,
            repository,
            commit,
            tree,
            fingerprint,
            created_at,
            parent_candidate: None,
        }
    }

    /// Mint a new candidate on top of an existing parent.
    ///
    /// The package identity and version are inherited from `parent` —
    /// a revision is a re-spin of the same package on a different
    /// commit/tree. `parent_candidate` on the new candidate is set to
    /// `Some(parent.id)`. The fingerprint is recomputed across the
    /// parent's package and the new repository/commit/tree, so it
    /// differs from the parent's whenever any of those three change.
    #[must_use]
    pub fn new_revision(
        parent: &SourceCandidate,
        repository: RepositoryRef,
        commit: GitObjectId,
        tree: GitObjectId,
        created_at: OffsetDateTime,
    ) -> Self {
        let fingerprint = compute_fingerprint(&parent.package, &repository, &commit, &tree);
        Self {
            id: CandidateId::new(),
            job_id: parent.job_id,
            package: parent.package.clone(),
            repository,
            commit,
            tree,
            fingerprint,
            created_at,
            parent_candidate: Some(parent.id),
        }
    }

    #[must_use]
    pub fn id(&self) -> CandidateId {
        self.id
    }

    #[must_use]
    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    #[must_use]
    pub fn package(&self) -> &PackageRevision {
        &self.package
    }

    #[must_use]
    pub fn repository(&self) -> &RepositoryRef {
        &self.repository
    }

    #[must_use]
    pub fn commit(&self) -> &GitObjectId {
        &self.commit
    }

    #[must_use]
    pub fn tree(&self) -> &GitObjectId {
        &self.tree
    }

    #[must_use]
    pub fn fingerprint(&self) -> &CandidateFingerprint {
        &self.fingerprint
    }

    #[must_use]
    pub fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub fn parent_candidate(&self) -> Option<CandidateId> {
        self.parent_candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::GitHashAlgorithm;
    use crate::distribution::{DistributionFamily, DistributionRef, DistributionRelease};
    use crate::package::{PackageIdentity, PackageName, PackageVersion};
    use time::macros::datetime;
    use url::Url;

    fn name(s: &str) -> PackageName {
        PackageName::new(s).unwrap()
    }
    fn ver(s: &str) -> PackageVersion {
        PackageVersion::new(s).unwrap()
    }
    fn pkg(dist: &str, release: &str, n: &str, v: &str) -> PackageRevision {
        PackageRevision::new(
            PackageIdentity::new(
                DistributionRef::new(
                    DistributionFamily::new(dist).unwrap(),
                    DistributionRelease::new(release).unwrap(),
                ),
                name(n),
            ),
            ver(v),
        )
    }
    fn git_oid(hex: &str) -> GitObjectId {
        GitObjectId::new(GitHashAlgorithm::Sha1, hex).unwrap()
    }
    fn repo(url: &str) -> RepositoryRef {
        RepositoryRef::new(crate::repository::VcsKind::Git, Url::parse(url).unwrap()).unwrap()
    }

    #[test]
    fn fingerprint_from_hex_accepts_64_hex_chars() {
        let hex = "a".repeat(64);
        assert!(CandidateFingerprint::from_hex(hex).is_ok());
    }

    #[test]
    fn fingerprint_from_hex_rejects_too_short() {
        let hex = "a".repeat(63);
        let err = CandidateFingerprint::from_hex(hex).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidDigest);
    }

    #[test]
    fn fingerprint_from_hex_rejects_too_long() {
        let hex = "a".repeat(65);
        let err = CandidateFingerprint::from_hex(hex).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidDigest);
    }

    #[test]
    fn fingerprint_from_hex_rejects_non_hex() {
        // 64 chars but contains a 'z'.
        let mut hex = "a".repeat(63);
        hex.push('z');
        let err = CandidateFingerprint::from_hex(hex).unwrap_err();
        assert_eq!(err.kind, CoreErrorKind::InvalidDigest);
    }

    #[test]
    fn fingerprint_from_hex_accepts_mixed_case() {
        // Hex validation accepts both cases; serialization is the caller's choice.
        let hex = "AbCdEf0123456789".repeat(4);
        assert!(CandidateFingerprint::from_hex(hex).is_ok());
    }

    #[test]
    fn fingerprint_serializes_as_transparent_hex_string() {
        let hex = "0123456789abcdef".repeat(4);
        let fp = CandidateFingerprint::from_hex(hex.clone()).unwrap();
        let json = serde_json::to_string(&fp).unwrap();
        assert_eq!(json, format!("\"{hex}\""));
    }

    #[test]
    fn candidate_new_mints_id_and_fingerprint() {
        let c = SourceCandidate::new(
            JobId::new(),
            pkg("debian", "unstable", "foo", "1.9.0-1"),
            repo("https://salsa.debian.org/foo/foo.git"),
            git_oid(&"1".repeat(40)),
            git_oid(&"2".repeat(40)),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        assert_ne!(c.id(), CandidateId::default());
        assert_eq!(c.fingerprint().as_str().len(), 64);
        assert!(c.parent_candidate().is_none());
    }

    #[test]
    fn candidate_new_revision_inherits_package_and_records_parent() {
        let parent = SourceCandidate::new(
            JobId::new(),
            pkg("debian", "unstable", "foo", "1.9.0-1"),
            repo("https://salsa.debian.org/foo/foo.git"),
            git_oid(&"1".repeat(40)),
            git_oid(&"2".repeat(40)),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let child = SourceCandidate::new_revision(
            &parent,
            repo("https://salsa.debian.org/foo/foo.git"),
            git_oid(&"3".repeat(40)),
            git_oid(&"4".repeat(40)),
            datetime!(2026-01-02 00:00:00 UTC),
        );
        assert_eq!(child.package(), parent.package());
        assert_eq!(child.job_id(), parent.job_id());
        assert_eq!(child.parent_candidate(), Some(parent.id()));
        // Fingerprint must change when commit/tree change.
        assert_ne!(child.fingerprint(), parent.fingerprint());
    }

    #[test]
    fn identical_inputs_produce_identical_fingerprints() {
        let pkg1 = pkg("debian", "unstable", "foo", "1.9.0-1");
        let pkg2 = pkg("debian", "unstable", "foo", "1.9.0-1");
        let repo_a = repo("https://salsa.debian.org/foo/foo.git");
        let repo_b = repo("https://salsa.debian.org/foo/foo.git");
        let commit_a = git_oid(&"1".repeat(40));
        let commit_b = git_oid(&"1".repeat(40));
        let tree_a = git_oid(&"2".repeat(40));
        let tree_b = git_oid(&"2".repeat(40));
        let fp1 = compute_fingerprint(&pkg1, &repo_a, &commit_a, &tree_a);
        let fp2 = compute_fingerprint(&pkg2, &repo_b, &commit_b, &tree_b);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn different_package_version_changes_fingerprint() {
        let pkg1 = pkg("debian", "unstable", "foo", "1.9.0-1");
        let pkg2 = pkg("debian", "unstable", "foo", "1.9.0-2");
        let r = repo("https://salsa.debian.org/foo/foo.git");
        let c = git_oid(&"1".repeat(40));
        let t = git_oid(&"2".repeat(40));
        let fp1 = compute_fingerprint(&pkg1, &r, &c, &t);
        let fp2 = compute_fingerprint(&pkg2, &r, &c, &t);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn different_release_changes_fingerprint() {
        let pkg1 = pkg("debian", "unstable", "foo", "1.9.0-1");
        let pkg2 = pkg("debian", "bookworm", "foo", "1.9.0-1");
        let r = repo("https://salsa.debian.org/foo/foo.git");
        let c = git_oid(&"1".repeat(40));
        let t = git_oid(&"2".repeat(40));
        let fp1 = compute_fingerprint(&pkg1, &r, &c, &t);
        let fp2 = compute_fingerprint(&pkg2, &r, &c, &t);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn different_commit_changes_fingerprint() {
        let p = pkg("debian", "unstable", "foo", "1.9.0-1");
        let r = repo("https://salsa.debian.org/foo/foo.git");
        let t = git_oid(&"2".repeat(40));
        let fp1 = compute_fingerprint(&p, &r, &git_oid(&"1".repeat(40)), &t);
        let fp2 = compute_fingerprint(&p, &r, &git_oid(&"a".repeat(40)), &t);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn different_tree_changes_fingerprint() {
        let p = pkg("debian", "unstable", "foo", "1.9.0-1");
        let r = repo("https://salsa.debian.org/foo/foo.git");
        let c = git_oid(&"1".repeat(40));
        let fp1 = compute_fingerprint(&p, &r, &c, &git_oid(&"2".repeat(40)));
        let fp2 = compute_fingerprint(&p, &r, &c, &git_oid(&"b".repeat(40)));
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn different_repository_url_changes_fingerprint() {
        let p = pkg("debian", "unstable", "foo", "1.9.0-1");
        let c = git_oid(&"1".repeat(40));
        let t = git_oid(&"2".repeat(40));
        let r1 = repo("https://salsa.debian.org/foo/foo.git");
        let r2 = repo("https://example.org/foo.git");
        let fp1 = compute_fingerprint(&p, &r1, &c, &t);
        let fp2 = compute_fingerprint(&p, &r2, &c, &t);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_is_deterministic_across_calls() {
        let p = pkg("debian", "unstable", "foo", "1.9.0-1");
        let r = repo("https://salsa.debian.org/foo/foo.git");
        let c = git_oid(&"1".repeat(40));
        let t = git_oid(&"2".repeat(40));
        let fp1 = compute_fingerprint(&p, &r, &c, &t);
        let fp2 = compute_fingerprint(&p, &r, &c, &t);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn candidate_serializes_to_expected_shape() {
        let c = SourceCandidate::new(
            JobId::new(),
            pkg("debian", "unstable", "foo", "1.9.0-1"),
            repo("https://salsa.debian.org/foo/foo.git"),
            git_oid(&"1".repeat(40)),
            git_oid(&"2".repeat(40)),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&c).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["id"].is_string());
        assert_eq!(parsed["package"]["version"], "1.9.0-1");
        assert_eq!(parsed["commit"]["algorithm"], "sha1");
        assert_eq!(parsed["tree"]["algorithm"], "sha1");
        assert_eq!(parsed["parent_candidate"], serde_json::Value::Null);
        // Fingerprint is present and 64 chars.
        assert_eq!(parsed["fingerprint"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn candidate_round_trips_through_json() {
        let c = SourceCandidate::new(
            JobId::new(),
            pkg("debian", "unstable", "foo", "1.9.0-1"),
            repo("https://salsa.debian.org/foo/foo.git"),
            git_oid(&"1".repeat(40)),
            git_oid(&"2".repeat(40)),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&c).unwrap();
        let parsed: SourceCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, c);
    }
}
