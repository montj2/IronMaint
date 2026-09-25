//! `ironmaint-evidence` — `MaintenanceEvidence` and gate evaluation.
//!
//! Distribution-neutral. "Does not know Debian or Fedora" (PHASE-0A.md §4.2).
//! Owns the evidence-and-gate vocabulary that backs the state machine:
//! [`MaintenanceEvidence`], [`MaintenanceGateDefinition`],
//! [`MaintenanceGateStage`], [`MaintenanceGateRequirement`],
//! [`MaintenanceGateEvaluation`], [`MaintenanceGateStatus`], and
//! `InvalidationRule` (stored only in Phase 0A; no engine applies it).
//!
//! ## Phase 0A.1 status
//!
//! Skeleton only. Evidence types, gate types, and `InvalidationRule` land
//! in 0A.3 alongside the state machine.

#![forbid(unsafe_code)]
