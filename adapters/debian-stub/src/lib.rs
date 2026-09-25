//! `debian-stub` — Phase 0A stub adapter for Debian.
//!
//! Implements [`DistributionAdapter`] for `DistributionFamily("debian")`.
//! The real `dpkg --compare-versions` semantics, Debian Policy
//! retrieval, and `lintian`/`sbuild` integration are explicitly out of
//! scope for Phase 0A (PHASE-0A.md §89).
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only. Trait implementation lands in 0A.4; conformance
//! verification in 0A.5.

#![forbid(unsafe_code)]
