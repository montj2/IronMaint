//! `policy/debian/authorities.v1.json` — the §19
//! Debian policy-knowledge baseline.
//!
//! The manifest is data, not behaviour. It is
//! bundled into the binary via `include_str!` at
//! compile time; no filesystem I/O at runtime.
//! 1D.2's `DebianPolicyDerivation` consumes the
//! loaded list.

use std::sync::LazyLock;

use ironmaint_policy::Authority;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The on-disk shape of
/// `policy/debian/authorities.v1.json`.
///
/// `pub` so `xtask/src/schemas.rs` can call
/// `schemars::schema_for!(DebianAuthorityManifestV1)`
/// for the §97 schema-snapshot tool. The
/// re-exports in `lib.rs` expose the loader
/// functions, not the struct itself.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DebianAuthorityManifestV1 {
    pub schema_version: u32,
    pub manifest_version: String,
    pub distribution: String,
    pub baseline_policy_version: String,
    pub authorities: Vec<Authority>,
}

/// The four §19 authorities, parsed and id-minted
/// once on first access. The `LazyLock` cache makes
/// `load_authorities()` deterministic across calls
/// within a process — the same `AuthorityId`s are
/// returned every time. This is load-bearing: the
/// runtime's `PolicyPlan` carries the IDs the loader
/// minted the first time, and any code that wants to
/// match a `Plan.authority` against a manifest entry
/// by ID needs to see the same IDs back.
#[allow(clippy::expect_used)] // The manifest is bundled via `include_str!` at
                              // compile time; the deserialiser only runs at first
                              // access. A `Result::Err` here means the JSON is
                              // structurally broken, which is a build-time
                              // defect the dev must fix before this binary can
                              // even start. The `expect` is the right tool for
                              // a precondition violation: it surfaces the
                              // diagnostic clearly and the binary is
                              // unrecoverable in any meaningful sense.
static LOADED: LazyLock<Vec<Authority>> = LazyLock::new(|| {
    let raw = include_str!("../../../policy/debian/authorities.v1.json");
    let manifest: DebianAuthorityManifestV1 = serde_json::from_str(raw)
        .expect("policy/debian/authorities.v1.json must be a valid manifest");
    manifest
        .authorities
        .into_iter()
        .map(|mut a| {
            a.id = ironmaint_core::AuthorityId::new();
            a
        })
        .collect()
});

/// Load the four §19 authorities bundled at
/// compile time. The `Authority::id` for each entry
/// is minted fresh on the first call and cached, so
/// every subsequent call returns the same IDs. The
/// cache is process-local and immutable after first
/// access; the manifest is data, not behaviour, and
/// a process restart is the only way to re-mint.
#[must_use]
pub fn load_authorities() -> &'static [Authority] {
    &LOADED
}

/// The baseline `Standards-Version` the
/// adapter treats as known good. 1D.2's
/// `delta_review` evaluator reads this.
#[must_use]
#[allow(clippy::expect_used)] // See `LOADED`: the manifest is bundled
                              // at compile time; an `Err` here is a
                              // build-time defect, not a runtime
                              // concern.
pub fn baseline_policy_version() -> &'static str {
    static BASELINE: LazyLock<String> = LazyLock::new(|| {
        let raw = include_str!("../../../policy/debian/authorities.v1.json");
        let m: DebianAuthorityManifestV1 = serde_json::from_str(raw)
            .expect("policy/debian/authorities.v1.json must be a valid manifest");
        m.baseline_policy_version
    });
    &BASELINE
}

/// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_policy::AuthorityClassification;

    #[test]
    fn the_manifest_has_four_authorities() {
        let authorities = load_authorities();
        assert_eq!(authorities.len(), 4);
    }

    #[test]
    fn debian_policy_is_normative_and_baseline() {
        let authorities = load_authorities();
        let policy = authorities
            .iter()
            .find(|a| a.name == "Debian Policy")
            .expect("Debian Policy is a §19 entry");
        assert_eq!(
            policy.classification,
            AuthorityClassification::NormativePolicy
        );
        assert_eq!(policy.version.as_deref(), Some("4.7.4.1"));
    }

    #[test]
    fn baseline_policy_version_is_4_7_4_1() {
        assert_eq!(baseline_policy_version(), "4.7.4.1");
    }

    #[test]
    fn developers_reference_is_best_practice_not_formal_specification() {
        // §19 explicit warning: "Never classify
        // the Developer's Reference globally as
        // `FormalSpecification`."
        let authorities = load_authorities();
        let dr = authorities
            .iter()
            .find(|a| a.name == "Developer's Reference")
            .expect("Developer's Reference is a §19 entry");
        assert_eq!(dr.classification, AuthorityClassification::BestPractice);
    }

    #[test]
    fn copyright_format_is_a_formal_specification() {
        let authorities = load_authorities();
        let cf = authorities
            .iter()
            .find(|a| a.name == "DEP-5 / copyright-format")
            .expect("copyright-format is a §19 entry");
        assert_eq!(
            cf.classification,
            AuthorityClassification::FormalSpecification
        );
    }

    #[test]
    fn bts_operational_protocol_is_a_distribution_procedure() {
        let authorities = load_authorities();
        let bts = authorities
            .iter()
            .find(|a| a.name == "BTS operational protocol")
            .expect("BTS operational protocol is a §19 entry");
        assert_eq!(
            bts.classification,
            AuthorityClassification::DistributionProcedure
        );
    }
}
