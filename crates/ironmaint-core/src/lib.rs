//! `ironmaint-core` — distribution-neutral value types.
//!
//! This crate is the **bottom of the Phase 0A dependency graph**. It must
//! not depend on any other workspace crate (PHASE-0A.md §5, §45). It owns
//! the type vocabulary that is genuinely distribution-neutral: identifiers
//! (UUIDv7 newtypes), distribution identity, package identity, repository
//! references, digests, and source candidates.
//!
//! ## What this crate explicitly does NOT contain
//!
//! Per PHASE-0A.md §45, this crate is "deliberately boring":
//!
//! - No Debian package fields, RPM tags, BTS semantics, or Bugzilla
//!   semantics.
//! - No build commands, policy evaluation, or state-transition logic.
//! - No network, MCP, or filesystem access.
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only. Value types, identifiers, and the candidate fingerprint
//! land in 0A.2.

#![forbid(unsafe_code)]
