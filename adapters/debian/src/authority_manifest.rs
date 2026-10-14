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
///
/// PHASE-1.1 PR 1.1.D (policy baseline correctness):
/// the pre-1.1.D struct had a single
/// `baseline_policy_version` field. The spec is
/// explicit (PHASE-1.md line 978) that the
/// `Standards-Version` comparator is the
/// `Standards-Version target` (= `"4.7.4"`), *not*
/// the Policy Manual document version
/// (= `"4.7.4.1"`). The new struct carries both:
/// `policy_document_version` is the document
/// version (for human readers / display), and
/// `standards_version_target` is the comparator
/// the runtime uses. The `Policy`'s
/// `version` in `authorities` is still
/// `4.7.4.1` (the document version the
/// authority *is*).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DebianAuthorityManifestV1 {
    pub schema_version: u32,
    pub manifest_version: String,
    pub distribution: String,
    /// The version of the Debian Policy *Manual
    /// document* (e.g. `"4.7.4.1"`). Display-only —
    /// the `Standards-Version` comparator is
    /// `standards_version_target`, not this.
    pub policy_document_version: String,
    /// The `Standards-Version` target the adapter
    /// treats as known good (e.g. `"4.7.4"`). The
    /// §22 `delta_review` evaluator compares the
    /// candidate's `debian/control`
    /// `Standards-Version` against this value.
    pub standards_version_target: String,
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

/// The `Standards-Version` target the adapter
/// treats as known good. 1D.2's `delta_review`
/// evaluator reads this.
///
/// PHASE-1.1 PR 1.1.D: this replaces the
/// pre-1.1.D `baseline_policy_version()` accessor,
/// which returned the Policy Manual *document*
/// version (4.7.4.1). The new accessor returns the
/// `Standards-Version` *target* (4.7.4) — the value
/// the candidate's `debian/control`
/// `Standards-Version` is compared against.
///
/// The pre-1.1.D accessor is kept as a deprecated
/// shim that delegates to the new one, so any
/// external caller (none today) does not break. The
/// `policy_delta` tool reads only
/// `standards_version_target()` after this PR.
#[must_use]
#[allow(clippy::expect_used)] // See `LOADED`: the manifest is bundled
// at compile time; an `Err` here is a
// build-time defect, not a runtime
// concern.
pub fn standards_version_target() -> &'static str {
    static TARGET: LazyLock<String> = LazyLock::new(|| {
        let raw = include_str!("../../../policy/debian/authorities.v1.json");
        let m: DebianAuthorityManifestV1 = serde_json::from_str(raw)
            .expect("policy/debian/authorities.v1.json must be a valid manifest");
        m.standards_version_target
    });
    &TARGET
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
        // The Debian Policy authority's version is
        // the *document* version (4.7.4.1), not the
        // `Standards-Version` target. The two are
        // different — the document version names
        // which Policy Manual the authority
        // references; the target names which
        // `Standards-Version` a candidate is
        // expected to declare.
        assert_eq!(policy.version.as_deref(), Some("4.7.4.1"));
    }

    #[test]
    fn standards_version_target_is_4_7_4() {
        // PHASE-1.1 PR 1.1.D: the `Standards-Version`
        // target is 4.7.4 (not 4.7.4.1, which is the
        // Policy Manual document version). A
        // candidate declaring `Standards-Version:
        // 4.7.4` is *at* the baseline; 4.7.4.1 is
        // *above* it.
        assert_eq!(standards_version_target(), "4.7.4");
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
