//! `ironmaint-adapter-api` — the adapter contract.
//!
//! Defines the [`DistributionAdapter`] root trait and one capability
//! "port" trait per adapter capability area (`VersioningPort`,
//! `PackageModelPort`, `PolicyPort`, `BuildPort`, `IssuePort`,
//! `ReleasePort`). Per PHASE-0A.md §45, "do not begin with one enormous
//! method. Use one root trait plus capability-specific traits."
//!
//! Adapters implement this trait; the core never imports any concrete
//! adapter. Adapter methods are `&self`-only and return plans/templates
//! rather than mutating workflow state.
//!
//! ## Dependency direction
//!
//! Depends on `ironmaint-core`, `ironmaint-evidence`, and
//! `ironmaint-policy`. Per PHASE-0A.md §4.5, this crate "should avoid
//! depending directly on `ironmaint-state`" — and it doesn't.
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only. Trait surface and plan types land in 0A.4.

#![forbid(unsafe_code)]
