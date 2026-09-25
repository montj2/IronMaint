//! Fedora build capability (§49).
//!
//! Build plan uses `fedora.build.mock`; QA plan uses
//! `fedora.qa.rpmlint`, `fedora.qa.spectool`, and
//! `fedora.test.functional`. Debian's stub uses different keys.

use std::sync::LazyLock;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_adapter_api::{
    AdapterError, BuildCapability, BuildPlan, CandidateContext, PlannedCheck, QaPlan,
};
use ironmaint_evidence::EvidenceKind;

static FEDORA_BUILD: LazyLock<FedoraBuild> = LazyLock::new(FedoraBuild::new);

pub struct FedoraBuild;

impl FedoraBuild {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FedoraBuild {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildCapability for FedoraBuild {
    fn build_plan(&self, _ctx: &CandidateContext) -> Result<BuildPlan, AdapterError> {
        Ok(BuildPlan::default().with_check(PlannedCheck {
            key: static_key("fedora.build.mock"),
            evidence_kind: EvidenceKind::Build,
            mandatory: true,
        }))
    }

    fn qa_plan(&self, _ctx: &CandidateContext) -> Result<QaPlan, AdapterError> {
        Ok(QaPlan::default()
            .with_check(PlannedCheck {
                key: static_key("fedora.qa.rpmlint"),
                evidence_kind: EvidenceKind::PackageQa,
                mandatory: true,
            })
            .with_check(PlannedCheck {
                key: static_key("fedora.qa.spectool"),
                evidence_kind: EvidenceKind::PackageQa,
                mandatory: false,
            })
            .with_check(PlannedCheck {
                key: static_key("fedora.test.functional"),
                evidence_kind: EvidenceKind::FunctionalTest,
                mandatory: true,
            }))
    }
}

#[must_use]
pub fn fedora_build() -> &'static FedoraBuild {
    &FEDORA_BUILD
}

#[allow(clippy::expect_used, clippy::unwrap_used)]
fn static_key(literal: &'static str) -> ToolCapabilityKey {
    ToolCapabilityKey::new(literal).expect("static literal must be a valid tool capability key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_plan_uses_mock() {
        let b = FedoraBuild::new();
        let plan = b.build_plan(&crate::fixtures::candidate_context()).unwrap();
        assert_eq!(plan.checks.len(), 1);
        assert_eq!(plan.checks[0].key.as_str(), "fedora.build.mock");
        assert!(plan.checks[0].mandatory);
    }

    #[test]
    fn qa_plan_uses_rpmlint_spectool_functional() {
        let b = FedoraBuild::new();
        let plan = b.qa_plan(&crate::fixtures::candidate_context()).unwrap();
        let keys: Vec<&str> = plan.checks.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "fedora.qa.rpmlint",
                "fedora.qa.spectool",
                "fedora.test.functional",
            ]
        );
    }

    #[test]
    fn mandatory_rpmlint_and_functional() {
        let b = FedoraBuild::new();
        let plan = b.qa_plan(&crate::fixtures::candidate_context()).unwrap();
        let rpmlint = plan
            .checks
            .iter()
            .find(|c| c.key.as_str() == "fedora.qa.rpmlint")
            .unwrap();
        let functional = plan
            .checks
            .iter()
            .find(|c| c.key.as_str() == "fedora.test.functional")
            .unwrap();
        assert!(rpmlint.mandatory);
        assert!(functional.mandatory);
    }
}
