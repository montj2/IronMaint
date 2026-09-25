//! `ironmaint-testkit` — shared fakes, fixtures, and the adapter
//! conformance suite.
//!
//! This crate holds the fixtures and conformance tests that both
//! `debian-stub` and `fedora-stub` are expected to satisfy. The shared
//! suite exists so a new distribution adapter can be validated against the
//! same contract that Debian and Fedora stubs pass (PHASE-0A.md §68).
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only. Fakes and conformance land in 0A.5.

#![forbid(unsafe_code)]
