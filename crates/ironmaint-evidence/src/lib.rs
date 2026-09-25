//! `ironmaint-evidence` — evidence, artifact, gate, and invalidation model.
//!
//! Distribution-neutral. "Does not know Debian or Fedora" (PHASE-0A.md §4.2).
//! Owns the evidence-and-gate vocabulary that backs the state machine:
//! [`Evidence`], [`EvidenceKind`], [`EvidenceStatus`], [`EvidenceProducer`],
//! [`ArtifactRef`], [`ArtifactKind`], [`GateDefinition`], [`GateStage`],
//! [`GateRequirement`], [`GateResult`], [`GateStatus`], [`InvalidationRule`],
//! [`ChangeDomain`], and [`EvidenceGraph`].
//!
//! ## Phase 0A.3 status
//!
//! All value types and the storage layer are in place. `EvidenceGraph`
//! stores evidence and gate results and offers candidate-scoped
//! lookups; sophisticated invalidation navigation lands later (see
//! §30: "Phase 0A should deliberately **not** implement cross-candidate
//! evidence reuse"). The state machine that consumes these types lives
//! in `ironmaint-state`.

#![forbid(unsafe_code)]
// Tests in this crate legitimately `.unwrap()` / `.expect()` on values
// whose constructors we've already validated by construction. The
// workspace `[lints.clippy]` table promotes `unwrap_used` and
// `expect_used` to `warn`, which combined with `-D warnings` would
// deny them in tests too. Allow them under `cfg(test)` only —
// production paths are still subject to the lint.
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

pub mod artifact;
pub mod evidence;
pub mod gate;
pub mod graph;
pub mod invalidation;
pub mod scope;

pub use artifact::{ArtifactKind, ArtifactRef};
pub use evidence::{Evidence, EvidenceKind, EvidenceProducer, EvidenceStatus};
pub use gate::{
    GateDefinition, GateRequirement, GateResult, GateStage, GateStatus, RequiredEvidenceStatus,
};
pub use graph::{EvidenceError, EvidenceGraph};
pub use invalidation::{ChangeDomain, InvalidationCause, InvalidationRule};
pub use scope::EvidenceScope;
