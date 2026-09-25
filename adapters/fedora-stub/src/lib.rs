//! `fedora-stub` — Phase 0A stub adapter for Fedora.
//!
//! Implements [`DistributionAdapter`] for `DistributionFamily("fedora")`.
//! Real RPM EVR comparison, Fedora guideline retrieval, and
//! `rpmlint`/`Mock`/`Koji`/`Bodhi` integration are explicitly out of scope
//! for Phase 0A (PHASE-0A.md §89).
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only. Trait implementation lands in 0A.4; conformance
//! verification in 0A.5.

#![forbid(unsafe_code)]
