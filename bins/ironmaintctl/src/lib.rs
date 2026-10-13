//! `ironmaintctl` — operator CLI for IronMaint, as a library so the
//! subcommand implementations are reachable from `tests/`.
//!
//! Phase 0B ships a single subcommand, `rebuild-projections`
//! (PHASE-0B.md §36). The binary in `main.rs` is a thin argument
//! parser over these.
//!
//! The lib target exists for one reason: an integration test can only
//! assert on a crate's behaviour if it can reach it, and the §36
//! escape hatch was untestable while its logic lived inside a `mod`
//! of a binary. `rebuild_smoke.rs` could spawn the binary, but could
//! not put a real job in front of it — so it tested the no-events
//! path, which was the one path that worked.

#![forbid(unsafe_code)]

pub mod rebuild;
