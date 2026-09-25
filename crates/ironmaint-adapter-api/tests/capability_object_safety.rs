//! Every capability trait must be object-safe (dyn-compatible).
//!
//! Spec §45 mandates that `DistributionAdapter` is a root trait
//! dispatchable through `&dyn`. Every capability trait exposed by
//! `ironmaint-adapter-api` must likewise be dyn-compatible, because
//! the root trait returns `&dyn Capability` for each one.

#![allow(dead_code)]

use ironmaint_adapter_api::{
    BuildCapability, DistributionAdapter, IssueCapability, PackageModelCapability,
    PolicyCapability, ReleaseCapability, VersioningCapability,
};

/// Each `fn _assert_trait_object(_: &dyn Trait)` body must compile.
/// If any capability trait gains a generic method, an associated
/// type without `+ Self: Sized`, or otherwise loses dyn-compatibility,
/// this file stops compiling — which is the test we want.
#[test]
fn capability_traits_are_dyn_safe() {
    fn _v(_: &dyn VersioningCapability) {}
    fn _p(_: &dyn PackageModelCapability) {}
    fn _po(_: &dyn PolicyCapability) {}
    fn _b(_: &dyn BuildCapability) {}
    fn _i(_: &dyn IssueCapability) {}
    fn _r(_: &dyn ReleaseCapability) {}
    fn _root(_: &dyn DistributionAdapter) {}
}
