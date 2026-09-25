//! Persistence contracts for IronMaint.
//!
//! This crate defines the trait surface (`IronMaintStore` facade plus
//! nine sub-traits) that backends (in-memory mocks, SQLite, etc.)
//! implement. All traits use native `async fn` in traits (stable on
//! MSRV 1.85; PHASE-0A.md §84) — no `async-trait` dependency.
//!
//! ## Sub-trait surface
//!
//! The nine sub-traits partition the durable state by domain area:
//!
//! | Sub-trait | Owns |
//! |---|---|
//! | [`event::EventStore`] | The per-job event log (`EventEnvelope<JobEvent>`) |
//! | [`projection::ProjectionStore`] | [`JobProjection`] cache |
//! | [`candidate::CandidateStore`] | [`SourceCandidate`] / [`ReleaseCandidate`] |
//! | [`evidence::EvidenceStore`] | [`Evidence`] records |
//! | [`gate::GateStore`] | [`GateDefinition`] / [`GateResult`] |
//! | [`obligation::ObligationStore`] | [`Obligation`] / [`ObligationStatus`] |
//! | [`operation::OperationStore`] | [`OperationRecord`] / lifecycle |
//! | [`artifact::ArtifactMetadataStore`] | [`ArtifactId`] metadata |
//! | [`workspace::WorkspaceMetadataStore`] | [`WorkspaceRevision`] / paths |
//!
//! The split mirrors the 0A domain modules: each sub-trait
//! corresponds to a bounded context's primary aggregate, kept small
//! enough that a backend implementation lives in one file.
//!
//! ## Backend-implementation guidance
//!
//! All trait methods take `&self` (not `&mut self`). Backends are
//! expected to manage interior mutability (`RwLock`, channel, etc.).
//! Optimistic concurrency on the projection cache uses the
//! `expected_version` parameter on [`projection::ProjectionStore::put`];
//! the engine already takes care of the bump on the in-memory
//! projection, so the store's job is to reject stale writes.

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
// `async fn` in traits is stable on MSRV 1.85 (PHASE-0A.md §84).
// The compiler's "use of async fn in public traits is discouraged"
// lint fires because Send bounds cannot be specified on the
// returned `impl Future` without an explicit return-type annotation;
// the auto-trait Send is sufficient for our Tokio-backed backends.
// See PHASE-0B.md §87: store/executor/runtime/MCP/daemon are async
// domains; this is intentional.
#![allow(async_fn_in_trait)]

pub mod artifact;
pub mod candidate;
pub mod envelope;
pub mod error;
pub mod event;
pub mod evidence;
pub mod gate;
pub mod mock;
pub mod obligation;
pub mod operation;
pub mod projection;
pub mod transaction;
pub mod workspace;

pub use envelope::EventEnvelope;
pub use error::{StoreError, StoreErrorKind};

pub use crate::artifact::ArtifactMetadataStore;
pub use crate::candidate::CandidateStore;
pub use crate::event::EventStore;
pub use crate::evidence::EvidenceStore;
pub use crate::gate::GateStore;
pub use crate::obligation::ObligationStore;
pub use crate::operation::OperationStore;
pub use crate::projection::ProjectionStore;
pub use crate::workspace::WorkspaceMetadataStore;

/// Aggregate facade over the nine sub-traits.
///
/// Backends (mock or SQLite) implement each sub-trait individually
/// and then expose `IronMaintStore` as a convenience trait that
/// bundles them. Callers that need only one sub-trait should depend
/// on it directly to keep their dependency surface minimal.
pub trait IronMaintStore:
    EventStore
    + ProjectionStore
    + CandidateStore
    + EvidenceStore
    + GateStore
    + ObligationStore
    + OperationStore
    + ArtifactMetadataStore
    + WorkspaceMetadataStore
{
}

impl<T> IronMaintStore for T where
    T: EventStore
        + ProjectionStore
        + CandidateStore
        + EvidenceStore
        + GateStore
        + ObligationStore
        + OperationStore
        + ArtifactMetadataStore
        + WorkspaceMetadataStore
{
}
