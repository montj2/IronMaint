//! `ironmaint-state` — the workflow state machine.
//!
//! Owns [`JobState`], [`JobProjection`], the [`TransitionEngine`], and
//! the [`StateTransitioned`] event. "Only crate allowed to determine
//! whether a job moves from state X to state Y" (PHASE-0A.md §4.4). No
//! adapter may bypass it; no agent may mutate [`JobState`] directly.
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only. The state machine and transition rules land in 0A.3.

#![forbid(unsafe_code)]
