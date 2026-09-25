//! `ironmaint-core` — distribution-neutral value types.
//!
//! This crate is the **bottom of the Phase 0A dependency graph**. It
//! must not depend on any other workspace crate (PHASE-0A.md §5, §45).
//! It owns the type vocabulary that is genuinely distribution-neutral:
//! identifiers (UUIDv7 newtypes), distribution identity, package
//! identity, repository references, digests, source candidates, jobs,
//! events, and the [`CoreError`] type.
//!
//! ## What this crate explicitly does NOT contain
//!
//! Per PHASE-0A.md §45, this crate is "deliberately boring":
//!
//! - No per-distribution package formats, version-compare semantics, or
//!   issue-tracker integrations.
//! - No build commands, policy evaluation, or state-transition logic.
//! - No network, MCP, or filesystem access.
//!
//! ## Phase 0A.2 status
//!
//! All value types are in place: identifiers, distribution, package,
//! repository, digest, event, job, schema, error, candidate. The
//! state machine itself lives in `ironmaint-state` (0A.3). The
//! adapter API lives in `ironmaint-adapter-api` (0A.4).

#![forbid(unsafe_code)]
// Tests in this crate legitimately `.unwrap()` / `.expect()` on values whose
// constructors we've already validated by construction. The workspace
// `[lints.clippy]` table promotes `unwrap_used` and `expect_used` to `warn`,
// which combined with `-D warnings` would deny them in tests too. Allow
// them under `cfg(test)` only — production paths are still subject to the
// lint.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::map_err_ignore,
    )
)]

pub mod candidate;
pub mod digest;
pub mod distribution;
pub mod error;
pub mod event;
pub mod identity;
pub mod issue_observation;
pub mod job;
pub mod package;
pub mod repository;
pub mod schema;

// Ergonomic re-exports so callers can use `ironmaint_core::JobId` etc.
// without naming the source module.
pub use candidate::{CandidateFingerprint, SourceCandidate};
pub use digest::{Digest, DigestAlgorithm, GitHashAlgorithm, GitObjectId};
pub use distribution::{DistributionFamily, DistributionRef, DistributionRelease};
pub use error::{CoreError, CoreErrorKind};
pub use event::{EventSource, MaintenanceEvent, MaintenanceEventType};
pub use identity::{
    ActorId, ApprovalId, ArtifactId, AuthorityId, CandidateId, DomainEventId, EvidenceId, GateId,
    IssueActionId, IssueProviderId, JobId, MaintenanceEventId, ObligationId, OperationId,
    ReleaseCandidateId,
};
pub use issue_observation::{IssueRef, IssueSnapshot, IssueState};
pub use job::{JobProjection, JobState, MaintenanceJob};
pub use package::{PackageIdentity, PackageName, PackageRevision, PackageVersion};
pub use repository::{RepoPath, RepositoryRef, VcsKind};
pub use schema::SchemaVersion;

// Schema-only newtypes used as `#[schemars(with = "...")]` overrides
// on wire fields whose underlying type (e.g. `time::OffsetDateTime`)
// doesn't have a `JsonSchema` impl we can legally provide under the
// orphan rule. The `uuid08` and `url` features on `schemars` handle
// `uuid::Uuid` and `url::Url` directly.
pub mod json_schema_impls;
