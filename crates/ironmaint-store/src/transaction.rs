//! Transactional boundary over [`IronMaintStore`].
//!
//! Most backends (SQLite, in-memory mocks) can group multiple
//! sub-trait operations into a single atomic unit. This facade
//! trait lets callers express that intent without depending on a
//! concrete backend.
//!
//! [`IronMaintStore`]: crate::IronMaintStore

use crate::artifact::ArtifactMetadataStore;
use crate::candidate::CandidateStore;
use crate::error::StoreError;
use crate::event::EventStore;
use crate::evidence::EvidenceStore;
use crate::gate::GateStore;
use crate::obligation::ObligationStore;
use crate::operation::OperationStore;
use crate::projection::ProjectionStore;
use crate::workspace::WorkspaceMetadataStore;

/// A scoped transactional view over an [`IronMaintStore`].
///
/// Backends that support transactions implement this trait; backends
/// that don't (e.g. simple key-value mocks) can provide a
/// "single-shot" implementation that simply forwards each call and
/// ignores `commit` / `rollback`.
///
/// [`IronMaintStore`]: crate::IronMaintStore
pub trait StoreTransaction:
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
    /// Commit all writes accumulated during this transaction.
    async fn commit(self: Box<Self>) -> Result<(), StoreError>;

    /// Discard all writes accumulated during this transaction.
    /// Default is a no-op.
    async fn rollback(self: Box<Self>) -> Result<(), StoreError> {
        Ok(())
    }
}
