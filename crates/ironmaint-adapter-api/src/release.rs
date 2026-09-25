//! Release capability (§52).
//!
//! Adapters describe release metadata and publication operations
//! as templates. The executor (Phase 0B) materializes each
//! `(PrivilegedOperationKind, String)` pair into a concrete
//! [`ironmaint_policy::PrivilegedOperation`] with proper ids and
//! approval requirements — that is what §52 means by "templates
//! become concrete publication plans only after candidate IDs and
//! approval requirements are created".

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ironmaint_core::DistributionRef;
use ironmaint_policy::PrivilegedOperationKind;

use crate::contexts::{PublicationContext, ReleaseContext};
use crate::error::AdapterError;

/// Pre-execution release-metadata bundle (§52).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseMetadataPlan {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl ReleaseMetadataPlan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    #[must_use]
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }
}

/// One operation the publication plan describes.
///
/// `summary` is the human-readable description of the operation;
/// the executor materializes it into a
/// [`ironmaint_policy::PrivilegedOperation`] with the appropriate
/// ids and approval requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlannedOperation {
    pub kind: PrivilegedOperationKind,
    pub summary: String,
}

impl PlannedOperation {
    #[must_use]
    pub fn new(kind: PrivilegedOperationKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
        }
    }
}

/// Pre-execution bundle for the publication plan (§52).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PublicationPlanTemplate {
    pub distribution: DistributionRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<PlannedOperation>,
    pub rationale: String,
}

impl PublicationPlanTemplate {
    #[must_use]
    pub fn new(distribution: DistributionRef, rationale: impl Into<String>) -> Self {
        Self {
            distribution,
            operations: Vec::new(),
            rationale: rationale.into(),
        }
    }

    #[must_use]
    pub fn with_operation(mut self, op: PlannedOperation) -> Self {
        self.operations.push(op);
        self
    }
}

/// Optional capability: describe release metadata + publication ops (§52).
pub trait ReleaseCapability: Send + Sync + 'static {
    /// Plan release metadata.
    fn release_metadata_plan(
        &self,
        context: &ReleaseContext,
    ) -> Result<ReleaseMetadataPlan, AdapterError>;

    /// Plan publication operations.
    fn publication_plan(
        &self,
        context: &PublicationContext,
    ) -> Result<PublicationPlanTemplate, AdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::{DistributionFamily, DistributionRelease};

    fn dref() -> DistributionRef {
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("sid").unwrap(),
        )
    }

    #[test]
    fn release_metadata_plan_builder_chain() {
        let plan = ReleaseMetadataPlan::new()
            .with_tag("debian/release")
            .with_notes("notes");
        assert_eq!(plan.tags, vec!["debian/release"]);
        assert_eq!(plan.notes.as_deref(), Some("notes"));
    }

    #[test]
    fn publication_plan_template_builder_chain() {
        let plan =
            PublicationPlanTemplate::new(dref(), "publish").with_operation(PlannedOperation::new(
                PrivilegedOperationKind::CanonicalRepositoryPush,
                "git push origin main",
            ));
        assert_eq!(plan.operations.len(), 1);
        assert_eq!(plan.rationale, "publish");
    }

    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn ReleaseCapability>();
    }
}
