//! Every capability trait must be object-safe (dyn-compatible).
//!
//! Spec §45 mandates that `DistributionAdapter` is a root trait
//! dispatchable through `&dyn`. Every capability trait exposed by
//! `ironmaint-adapter-api` must likewise be dyn-compatible, because
//! the root trait returns `&dyn Capability` for each one.

use ironmaint_adapter_api::{
    BuildCapability, DistributionAdapter, IssueCapability, PackageModelCapability,
    PolicyCapability, ReleaseCapability, VersioningCapability,
};

/// Each `fn accepts(_: &dyn Trait)` body must compile. If any
/// capability trait gains a generic method, an associated type
/// without `+ Self: Sized`, or otherwise loses dyn-compatibility,
/// this file stops compiling — which is the test we want.
///
/// The functions are never *called*; what is under test is whether
/// `&dyn Trait` names a type at all. This file used to carry a
/// file-level `#![allow(dead_code)]` to permit that, which was the
/// wrong instrument twice over: it exempted the *whole file*, so any
/// genuinely dead item added here later would have been invisible,
/// and the lint suppression was standing in for the assertion.
///
/// Each binding below coerces one function to a fn-pointer type
/// naming its trait, which is what makes it a live item — the
/// coercion is the check. They are separate `let`s rather than one
/// tuple because `clippy::type_complexity` reads a seven-element
/// tuple of `fn(&dyn Trait)` types as a complex type, and the fix
/// for that warning would be a type alias, which is more machinery
/// than the assertion is worth.
#[test]
fn capability_traits_are_dyn_safe() {
    fn versioning(_: &dyn VersioningCapability) {}
    fn package_model(_: &dyn PackageModelCapability) {}
    fn policy(_: &dyn PolicyCapability) {}
    fn build(_: &dyn BuildCapability) {}
    fn issue(_: &dyn IssueCapability) {}
    fn release(_: &dyn ReleaseCapability) {}
    fn root(_: &dyn DistributionAdapter) {}

    let _: fn(&dyn VersioningCapability) = versioning;
    let _: fn(&dyn PackageModelCapability) = package_model;
    let _: fn(&dyn PolicyCapability) = policy;
    let _: fn(&dyn BuildCapability) = build;
    let _: fn(&dyn IssueCapability) = issue;
    let _: fn(&dyn ReleaseCapability) = release;
    let _: fn(&dyn DistributionAdapter) = root;
}
