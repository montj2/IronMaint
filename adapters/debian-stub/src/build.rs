//! Debian build capability (§49).
//!
//! Build plan uses `debian.build.sbuild`; QA plan layers
//! `debian.qa.lintian`, `debian.qa.piuparts`, and
//! `debian.test.autopkgtest`. Fedora's stub uses different keys
//! (`fedora.build.mock`, `fedora.qa.rpmlint`).

use std::sync::LazyLock;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_adapter_api::{
    AdapterError, BuildCapability, BuildPlan, CandidateContext, PlannedCheck, QaPlan,
};
use ironmaint_evidence::EvidenceKind;

static DEBIAN_BUILD: LazyLock<DebianBuild> = LazyLock::new(DebianBuild::new);

pub struct DebianBuild;

impl DebianBuild {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianBuild {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildCapability for DebianBuild {
    fn build_plan(&self, _ctx: &CandidateContext) -> Result<BuildPlan, AdapterError> {
        Ok(BuildPlan::default().with_check(PlannedCheck {
            key: static_key("debian.build.sbuild"),
            evidence_kind: EvidenceKind::Build,
            mandatory: true,
        }))
    }

    fn qa_plan(&self, _ctx: &CandidateContext) -> Result<QaPlan, AdapterError> {
        Ok(QaPlan::default()
            .with_check(PlannedCheck {
                key: static_key("debian.qa.lintian"),
                evidence_kind: EvidenceKind::PackageQa,
                mandatory: true,
            })
            .with_check(PlannedCheck {
                key: static_key("debian.qa.piuparts"),
                evidence_kind: EvidenceKind::PackageQa,
                mandatory: false,
            })
            .with_check(PlannedCheck {
                key: static_key("debian.test.autopkgtest"),
                evidence_kind: EvidenceKind::FunctionalTest,
                mandatory: true,
            }))
    }
}

#[must_use]
pub fn debian_build() -> &'static DebianBuild {
    &DEBIAN_BUILD
}

/// Construct a [`ToolCapabilityKey`] from a static literal.
///
/// All keys in this module are compile-time constants that satisfy
/// the grammar; the panic is unreachable. Narrow `expect_used`
/// allowance keeps the panic behaviour localised to this helper.
#[allow(clippy::expect_used, clippy::unwrap_used)]
fn static_key(literal: &'static str) -> ToolCapabilityKey {
    ToolCapabilityKey::new(literal).expect("static literal must be a valid tool capability key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_plan_uses_sbuild() {
        let b = DebianBuild::new();
        let plan = b.build_plan(&crate::fixtures::candidate_context()).unwrap();
        assert_eq!(plan.checks.len(), 1);
        assert_eq!(plan.checks[0].key.as_str(), "debian.build.sbuild");
        assert!(plan.checks[0].mandatory);
    }

    #[test]
    fn qa_plan_uses_lintian_piuparts_autopkgtest() {
        let b = DebianBuild::new();
        let plan = b.qa_plan(&crate::fixtures::candidate_context()).unwrap();
        let keys: Vec<&str> = plan.checks.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "debian.qa.lintian",
                "debian.qa.piuparts",
                "debian.test.autopkgtest",
            ]
        );
    }

    #[test]
    fn mandatory_lintian_and_autopkgtest() {
        let b = DebianBuild::new();
        let plan = b.qa_plan(&crate::fixtures::candidate_context()).unwrap();
        let lintian = plan
            .checks
            .iter()
            .find(|c| c.key.as_str() == "debian.qa.lintian")
            .unwrap();
        let autopkgtest = plan
            .checks
            .iter()
            .find(|c| c.key.as_str() == "debian.test.autopkgtest")
            .unwrap();
        assert!(lintian.mandatory);
        assert!(autopkgtest.mandatory);
    }
}
