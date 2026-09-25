//! `ironmaint-policy` — Authority and obligation vocabulary.
//!
//! Owns [`Authority`], [`AuthorityClassification`], [`PolicyReference`],
//! [`PolicyBaseline`], [`MaintenanceObligation`], and
//! [`MaintenanceObligationTemplate`]. Distribution-neutral: this crate does
//! **not** retrieve policy documents in Phase 0A (PHASE-0A.md §4.3).
//!
//! ## Dependency direction
//!
//! Per the §5 dependency graph, `ironmaint-policy` depends only on
//! `ironmaint-core`. It does **not** depend on `ironmaint-evidence` even
//! though `MaintenanceObligation` references `EvidenceId`; that coupling is
//! intentional — the obligation type holds evidence IDs as opaque
//! identifiers, not as evidence values. The state crate pulls both.
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only.

#![forbid(unsafe_code)]
