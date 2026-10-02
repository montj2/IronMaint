//! Store conformance suite — one script, run against every backend.
//!
//! [`assert_store_conformance`] is the single public entry point. It is
//! **generic over `S: IronMaintStore`** rather than taking a trait
//! object, because the sub-traits declare `async fn` and so are not
//! dyn-compatible; two backends of different types call the same
//! function and that is the only way to share it.
//!
//! This is a *different and smaller* job from [`super::conformance`],
//! which is adapter conformance. That one checks that a
//! `DistributionAdapter` honours §68. This one checks that two
//! implementations of the same store trait agree with each other.
//!
//! ## Why the error cases are the load-bearing half
//!
//! The happy path cannot lie. Every backend that stores a candidate and
//! returns it looks the same, so a round-trip test passes against
//! anything that stores things.
//!
//! A mock that returns `Ok` where SQLite returns `Err` is not a
//! simplification — it is a hole with a passing test over it, and the
//! gap is invisible to every test written against the mock. That is
//! not hypothetical: it is how defect instances 9 and 11 of the
//! fourteen in `doc/SEAM-VERIFICATION.md` §1.1 survived, because every
//! pre-existing capture test ran on `MockStore` and the backend the
//! daemon actually runs was never in the loop. §36 of the spec treats
//! "mock and SQLite must agree" as a DoD item, and the mock returning
//! `Ok` on a duplicate fingerprint is exactly that: a `UNIQUE`
//! constraint on one side and a silent index overwrite on the other.
//!
//! **So the cases below are weighted towards what a backend does when
//! it cannot do the thing**, and each one is written to fail on either
//! half of the answer — the wrong `Ok`, or the right `Err` under the
//! wrong [`StoreErrorKind`].
//!
//! ## Distribution neutrality
//!
//! Like [`super::conformance`], this module contains no `debian` /
//! `fedora` / `ubuntu` / `rhel` literal. The only distribution-shaped
//! string it sees is one this module supplies itself, to the fixtures
//! it builds.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented
)]

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
    SourceCandidate, VcsKind,
};
use ironmaint_store::{IronMaintStore, StoreError, StoreErrorKind};
use time::macros::datetime;
use url::Url;

/// Run every store-conformance case against `store`.
///
/// Each case is a separate `async fn` rather than inline blocks, so a
/// failure names the contract it broke instead of a line number in a
/// long function. The order is happy-path first, then the error cases:
/// when a backend is broken, the first thing worth knowing is *which*
/// of the two it is.
pub async fn assert_store_conformance<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    candidate_round_trip_is_lossless(store).await;
    listing_returns_exactly_what_was_written(store).await;
    re_putting_the_same_candidate_is_an_upsert(store).await;
    duplicate_fingerprint_is_rejected(store).await;
    a_fingerprint_is_shared_across_jobs(store).await;
    an_unknown_candidate_is_not_found(store).await;
    an_unknown_fingerprint_is_absent_not_an_error(store).await;
    an_unknown_job_lists_nothing(store).await;
}

/// A candidate survives `put` → `get` byte-for-byte.
///
/// The floor. If a backend cannot do this nothing else in the suite
/// means anything, so it runs first.
async fn candidate_round_trip_is_lossless<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    let job = JobId::new();
    let candidate = source_candidate(job, '1', '1');
    create_job(store, job).await;
    let id = store
        .put_source_candidate(&candidate)
        .await
        .expect("a fresh candidate must be storable");

    assert_eq!(
        id,
        candidate.id(),
        "put must return the candidate's own id, not mint one"
    );

    let got = store
        .get_source_candidate(id)
        .await
        .expect("a just-stored candidate must be readable");
    assert_eq!(
        got, candidate,
        "the round trip must be lossless: a backend that drops or \
         normalises a field here will make every later comparison a lie"
    );
}

/// `list_source_candidates_for_job` returns exactly the written set.
///
/// Ordering is deliberately *not* asserted. SQLite returns rows in
/// whatever order the query plan yields and MockStore returns
/// insertion order; pinning the order would make the suite assert on
/// an implementation detail neither backend promises. Membership is the
/// contract, sequence is not.
async fn listing_returns_exactly_what_was_written<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    let job = JobId::new();
    let other_job = JobId::new();
    create_job(store, job).await;
    create_job(store, other_job).await;

    let mut written = Vec::new();
    for tag in ['1', '2', '3'] {
        let candidate = source_candidate(job, '2', tag);
        store.put_source_candidate(&candidate).await.expect("store");
        written.push(candidate.id());
    }
    // A candidate for a different job, to prove the filter is applied.
    // The tag differs: see `a_fingerprint_is_shared_across_jobs` — the
    // same content under a different job is the *same* fingerprint,
    // and the store rightly refuses it.
    store
        .put_source_candidate(&source_candidate(other_job, '2', '4'))
        .await
        .expect("store");

    let mut listed = store
        .list_source_candidates_for_job(job)
        .await
        .expect("list");
    listed.sort();
    written.sort();

    assert_eq!(
        listed, written,
        "the listing must be exactly this job's candidates and not one \
         more; a backend that leaks another job's rows here would let a \
         gate read evidence it has no business seeing"
    );
}

/// Writing the *same* candidate twice succeeds, and updates in place.
///
/// The twin of the next case, and the one more likely to be got
/// wrong. `candidate_id` is the conflict target — SQLite declares
/// `ON CONFLICT(candidate_id) DO UPDATE` — so re-writing a candidate
/// is an upsert and must be `Ok`. A backend that rejected it would
/// break every retry, and a backend that *minted a second row* would
/// silently duplicate a candidate that §30 says is immutable.
async fn re_putting_the_same_candidate_is_an_upsert<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    let job = JobId::new();
    let candidate = source_candidate(job, '3', '1');
    create_job(store, job).await;
    let first = store
        .put_source_candidate(&candidate)
        .await
        .expect("first put");
    let second = store
        .put_source_candidate(&candidate)
        .await
        .expect("re-putting the same candidate is an upsert, not a conflict");

    assert_eq!(first, second, "an upsert returns the same id");
    assert_eq!(
        store
            .list_source_candidates_for_job(job)
            .await
            .expect("list")
            .len(),
        1,
        "an upsert must not mint a second row: a `SourceCandidate` is \
         immutable, so a second row for the same id is a duplicate that \
         nothing downstream is prepared to notice"
    );
}

/// Two candidates sharing a fingerprint are rejected.
///
/// **This is the case that found defect instances 9 and 11.** A
/// `SourceCandidate`'s fingerprint is a BLAKE3 over its content, and
/// `migrations/0001_initial.sql` declares `fingerprint TEXT NOT NULL
/// UNIQUE` — re-capturing an unchanged tree must not mint a second
/// candidate.
///
/// `MockStore` used to accept this: it did
/// `source_by_fp.insert(fp, id)` unconditionally, overwriting the
/// index and leaving `find_source_by_fingerprint` pointing at the
/// newer candidate. Every test that exercised capture used the mock,
/// so the divergence had a passing test over it.
///
/// The second candidate differs only in `candidate_id` — a fresh
/// UUIDv7 over identical content — which is exactly the shape a
/// re-capture produces.
async fn duplicate_fingerprint_is_rejected<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    let job = JobId::new();
    let first = source_candidate(job, '4', '1');
    let second = source_candidate(job, '4', '1');
    assert_eq!(
        first.fingerprint(),
        second.fingerprint(),
        "the fixture is wrong: identical inputs must fingerprint alike"
    );
    assert_ne!(
        first.id(),
        second.id(),
        "the fixture is wrong: two candidates must not share an id, or \
         this case is re-testing the upsert above"
    );

    create_job(store, job).await;

    store.put_source_candidate(&first).await.expect("first put");

    let outcome = store.put_source_candidate(&second).await;
    let error = match outcome {
        Ok(id) => panic!(
            "a duplicate fingerprint must be rejected, but the store \
             accepted candidate {id} and the index now resolves that \
             fingerprint to the newer candidate. A backend that returns \
             Ok here is not a simplification of the other one -- it is \
             a second, invisible divergence."
        ),
        Err(e) => e,
    };

    assert!(
        matches!(
            error.kind,
            StoreErrorKind::Conflict | StoreErrorKind::Backend
        ),
        "a duplicate fingerprint must surface as Conflict or Backend, \
         not {:?} -- an error kind no caller can branch on is an error \
         kind no caller will handle",
        error.kind
    );

    // The index must still resolve to the original. A backend that
    // rejects the insert *and* clobbers the mapping has lost a row
    // rather than refused a write, which is worse.
    assert_eq!(
        store
            .find_source_by_fingerprint(first.fingerprint())
            .await
            .expect("lookup"),
        Some(first.id()),
        "the refused insert must leave the original candidate \
         reachable; losing it would be data loss rather than a refusal"
    );
}

/// Two jobs capturing the same content collide, and that is correct.
///
/// **This case was written because the suite refused to build its own
/// fixtures without it.** `listing_returns_exactly_what_was_written`
/// originally used the same content for two different jobs to prove
/// the per-job filter worked, and the duplicate-fingerprint guard
/// rejected it — correctly.
///
/// `compute_fingerprint` hashes family, release, source name, version,
/// repository URL, commit and tree. **`JobId` is not among them**, and
/// `migrations/0001_initial.sql` declares `fingerprint` `UNIQUE` over
/// the whole table rather than per job. So a `SourceCandidate` is a
/// property of a *source tree*, not of a job: two jobs that capture
/// the same tree are contending for one candidate, and the second
/// write is refused.
///
/// That is a deliberate-looking consequence of two independent design
/// choices, neither of which mentions the other, and **nothing in the
/// workspace asserted it until this case existed.** Whether it is the
/// intended behaviour is not this suite's call — a Phase 1 adapter
/// that lets two jobs point at one upstream tree needs to know. The
/// suite pins the behaviour so that a change to it is a decision
/// rather than an accident.
async fn a_fingerprint_is_shared_across_jobs<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    let first_job = JobId::new();
    let second_job = JobId::new();
    let first = source_candidate(first_job, '5', '1');
    let second = source_candidate(second_job, '5', '1');

    assert_eq!(
        first.fingerprint(),
        second.fingerprint(),
        "`JobId` is not an input to the fingerprint, so identical content \
         under two jobs must hash alike"
    );

    create_job(store, first_job).await;
    create_job(store, second_job).await;

    store.put_source_candidate(&first).await.expect("first put");

    let error = store
        .put_source_candidate(&second)
        .await
        .expect_err("the UNIQUE constraint is global, not per job");
    assert!(
        matches!(
            error.kind,
            StoreErrorKind::Conflict | StoreErrorKind::Backend
        ),
        "expected a conflict-shaped error, got {:?} ({})",
        error.kind,
        error.detail
    );
    assert_eq!(
        store
            .find_source_by_fingerprint(first.fingerprint())
            .await
            .expect("lookup"),
        Some(first.id()),
        "the winning candidate is the one that got there first; the \
         refused one must not displace it"
    );
}

/// Reading a candidate that was never written is `NotFound`.
///
/// The error case in its plainest form. A backend that returns a
/// default-constructed candidate here would let a gate run against
/// evidence that does not exist.
async fn an_unknown_candidate_is_not_found<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    let ghost = source_candidate(JobId::new(), '6', '1');
    let error = store
        .get_source_candidate(ghost.id())
        .await
        .expect_err("a candidate that was never written must not be readable");

    assert_eq!(
        error.kind,
        StoreErrorKind::NotFound,
        "expected NotFound for an unstored candidate, got {:?} ({})",
        error.kind,
        error.detail
    );
}

/// An unknown fingerprint is `Ok(None)`, not an error.
///
/// The mirror of the case above, and the reason this pair is written
/// together. "Absent" and "absent *and* something went wrong" are
/// different answers, and a backend that conflates them turns every
/// cache miss into a failure.
async fn an_unknown_fingerprint_is_absent_not_an_error<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    let absent = CandidateFingerprint::from_hex("f".repeat(64))
        .expect("a 64-char hex string is a well-formed fingerprint");

    assert_eq!(
        store
            .find_source_by_fingerprint(&absent)
            .await
            .expect("an absent fingerprint is a normal answer, not an error"),
        None,
        "an unknown fingerprint must resolve to None"
    );
}

/// Listing a job with no candidates returns an empty list.
///
/// Not an error. The capture flow calls this before the first
/// candidate exists on every job it ever sees.
async fn an_unknown_job_lists_nothing<S>(store: &S)
where
    S: IronMaintStore + ?Sized,
{
    assert!(
        store
            .list_source_candidates_for_job(JobId::new())
            .await
            .expect("a job with no candidates is not an error")
            .is_empty(),
        "a job that has never captured anything must list nothing"
    );
}

/// Create the job that a candidate will hang off.
///
/// **This exists because of the second divergence the suite found.**
/// `source_candidates.job_id` is a `FOREIGN KEY` in
/// `migrations/0001_initial.sql`, so SQLite refuses a candidate whose
/// job has never been written — `open_in_memory` does not, because it
/// skips migrations and therefore has no constraint to enforce.
/// `MockStore` has no referential integrity at all and accepts the
/// orphan.
///
/// The script satisfies the constraint rather than asserting against
/// it: SQLite's behaviour is the one the daemon runs, and a case
/// written as "the mock must also reject orphans" would be a
/// behaviour change to `MockStore` with a blast radius across every
/// test that builds a candidate without a job, which is a decision
/// rather than a conformance fix. **The gap is recorded in
/// `doc/DEBT.md` (D-21) instead of being quietly papered over here.**
///
/// The cost of satisfying it rather than testing it is that this
/// helper quietly becomes a precondition of every case below, so it is
/// called at the top of each one rather than once in
/// [`assert_store_conformance`] — a case that creates its own job
/// would then be indistinguishable from one that forgot.
async fn create_job<S>(store: &S, job: JobId)
where
    S: IronMaintStore + ?Sized,
{
    let at = datetime!(2026-01-01 00:00:00 UTC);
    let projection = JobProjection {
        job: MaintenanceJob::new(job, package_identity(), MaintenanceEventId::new(), at),
        state: JobState::EventDetected,
        active_candidate: None,
        version: 0,
        updated_at: at,
    };
    store
        .put_projection(&projection, 0)
        .await
        .expect("a brand-new job must be storable at version 0");
}

/// The package identity every fixture candidate carries.
fn package_identity() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("conformance").expect("a valid family"),
            DistributionRelease::new("conformance").expect("a valid release"),
        ),
        PackageName::new("store-conformance").expect("a valid package name"),
    )
}

/// A `SourceCandidate` that is unique in `(ns, tag)` and collides otherwise.
///
/// **The cases share one store, so this fixture has to keep them
/// apart.** `compute_fingerprint` hashes the source content and
/// `migrations/0001_initial.sql` makes it `UNIQUE` across the whole
/// table, so a candidate built by one case with the same `(ns, tag)` as
/// a candidate built by an earlier one is a duplicate the store
/// correctly refuses — and the failure reads as a conformance
/// violation when it is really two test fixtures colliding. Each case
/// therefore takes its own `ns`; `tag` varies the content *within* a
/// case, which is what the listing case and the duplicate cases need.
///
/// `ns` becomes the commit hash and `tag` the tree hash, both 40 hex
/// characters, so both must be hex digits.
///
/// Two calls with the same `(ns, tag)` and different `job`s produce
/// the same fingerprint and different ids — the shape a re-capture
/// makes, and what `a_fingerprint_is_shared_across_jobs` pins.
fn source_candidate(job: JobId, ns: char, tag: char) -> SourceCandidate {
    let repo = RepositoryRef::new(
        VcsKind::Git,
        Url::parse("https://example.invalid/store-conformance.git").expect("static URL parses"),
    )
    .expect("an https URL is a valid repository reference");

    let at = datetime!(2026-01-01 00:00:00 UTC);

    SourceCandidate::new(
        job,
        PackageRevision::new(
            package_identity(),
            PackageVersion::new("1.0.0").expect("a valid version"),
        ),
        repo,
        GitObjectId::new(GitHashAlgorithm::Sha1, ns.to_string().repeat(40))
            .expect("a hex digit repeated 40 times is a valid object id"),
        GitObjectId::new(GitHashAlgorithm::Sha1, tag.to_string().repeat(40))
            .expect("a hex digit repeated 40 times is a valid object id"),
        at,
    )
}

/// Unused today, and kept because the error cases should compare the
/// *kind* rather than the message.
///
/// Backend error details are backend-specific by design — the spec
/// says a backend SHOULD NOT echo untrusted input into `detail` — so
/// a conformance assertion must never match on message text. This
/// helper exists to make the temptation visible at the point of use.
#[allow(dead_code)]
fn assert_error_kind(actual: &StoreError, expected: StoreErrorKind, context: &str) {
    assert_eq!(
        actual.kind, expected,
        "{context}: expected {expected:?}, got {:?} ({})",
        actual.kind, actual.detail
    );
}
