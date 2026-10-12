//! `policy/debian/authorities.v1.json` — the §19
//! Debian policy-knowledge baseline.
//!
//! The manifest is data, not behaviour. It is
//! bundled into the binary via `include_str!` at
//! compile time; no filesystem I/O at runtime.
//! 1D.2's `DebianPolicyDerivation` consumes the
//! loaded list.

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

/// Load the four §19 authorities bundled at
/// compile time. The `Authority::id` is re-minted
/// fresh per call so multiple manifest entries
/// (and multiple invocations) do not share an
/// id; see [`default_authority_id`] in
/// `ironmaint-policy::authority` for the
/// rationale.
pub fn load_authorities() -> Vec<Authority> {
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
}

/// The baseline `Standards-Version` the
/// adapter treats as known good. 1D.2's
/// `delta_review` evaluator reads this.
pub fn baseline_policy_version() -> String {
    let raw = include_str!("../../../policy/debian/authorities.v1.json");
    let m: DebianAuthorityManifestV1 = serde_json::from_str(raw)
        .expect("policy/debian/authorities.v1.json must be a valid manifest");
    m.baseline_policy_version
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
        assert_eq!(policy.classification, AuthorityClassification::NormativePolicy);
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
        assert_eq!(cf.classification, AuthorityClassification::FormalSpecification);
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
