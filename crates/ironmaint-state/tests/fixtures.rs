//! Shared test fixtures for the engine scenario tests.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::map_err_ignore
)]

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease, JobProjection,
    MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName,
};
use time::macros::datetime;

#[must_use]
pub fn family() -> DistributionFamily {
    DistributionFamily::new("debian").unwrap()
}

#[must_use]
pub fn release() -> DistributionRelease {
    DistributionRelease::new("unstable").unwrap()
}

#[must_use]
pub fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(family(), release()),
        PackageName::new("ironmaint-fixture").unwrap(),
    )
}

#[must_use]
pub fn fp() -> CandidateFingerprint {
    CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
}

#[must_use]
pub fn job(state: ironmaint_core::JobState, version: u64) -> JobProjection {
    JobProjection {
        job: MaintenanceJob::new(
            ironmaint_core::JobId::new(),
            package(),
            MaintenanceEventId::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        ),
        state,
        active_candidate: None,
        version,
        updated_at: datetime!(2026-01-02 00:00:00 UTC),
    }
}
