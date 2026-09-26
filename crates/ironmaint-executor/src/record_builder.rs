//! `ToolDefinitionRecord` — concrete struct that implements
//! [`ToolDefinition`]. Most callers should use this rather than
//! implementing the trait by hand. (PHASE-0B.md §27.)

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;

use crate::limits::{ExecutionClass, ExecutionLimits};
use crate::normalizer::ResultNormalizer;
use crate::registry::ToolDefinition;

/// Concrete [`ToolDefinition`] built from spec §27 fields. Construct
/// via [`ToolDefinitionRecord::new`], then optionally call
/// [`ToolDefinitionRecord::with_normalizer`] to attach a result
/// normalizer.
#[derive(Clone)]
pub struct ToolDefinitionRecord {
    key: ToolCapabilityKey,
    executable: PathBuf,
    fixed_args: Vec<OsString>,
    class: ExecutionClass,
    limits: ExecutionLimits,
    normalizer: Option<Arc<dyn ResultNormalizer>>,
}

impl ToolDefinitionRecord {
    /// Build a record without a normalizer. Call
    /// [`Self::with_normalizer`] to attach one.
    #[must_use]
    pub fn new(
        key: ToolCapabilityKey,
        executable: impl Into<PathBuf>,
        fixed_args: Vec<OsString>,
        class: ExecutionClass,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            key,
            executable: executable.into(),
            fixed_args,
            class,
            limits,
            normalizer: None,
        }
    }

    /// Attach a result normalizer. Returns `self` for chaining.
    #[must_use]
    pub fn with_normalizer(mut self, normalizer: Arc<dyn ResultNormalizer>) -> Self {
        self.normalizer = Some(normalizer);
        self
    }
}

impl ToolDefinition for ToolDefinitionRecord {
    fn key(&self) -> &ToolCapabilityKey {
        &self.key
    }

    fn executable(&self) -> &Path {
        &self.executable
    }

    fn fixed_args(&self) -> &[OsString] {
        &self.fixed_args
    }

    fn class(&self) -> ExecutionClass {
        self.class
    }

    fn limits(&self) -> ExecutionLimits {
        self.limits.clone()
    }

    fn normalizer(&self) -> Option<Arc<dyn ResultNormalizer>> {
        self.normalizer.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_evidence::EvidenceStatus;
    use std::time::Duration;

    use crate::normalizer::{NormalizationError, NormalizedResult};

    fn validate_key() -> ToolCapabilityKey {
        ToolCapabilityKey::new("synthetic.build.validate").unwrap()
    }

    #[test]
    fn record_returns_each_field() {
        let r = ToolDefinitionRecord::new(
            validate_key(),
            PathBuf::from("/usr/bin/ironmaint-fixture"),
            vec![OsString::from("--validate")],
            ExecutionClass::Check,
            ExecutionLimits {
                timeout: Duration::from_secs(5),
                stdout_max_bytes: 4096,
                stderr_max_bytes: 2048,
            },
        );
        assert_eq!(r.key().as_str(), "synthetic.build.validate");
        assert_eq!(r.executable(), Path::new("/usr/bin/ironmaint-fixture"));
        assert_eq!(r.fixed_args(), &[OsString::from("--validate")]);
        assert_eq!(r.class(), ExecutionClass::Check);
        assert_eq!(r.limits().timeout, Duration::from_secs(5));
        assert!(r.normalizer().is_none());
    }

    #[test]
    fn record_with_normalizer_round_trips() {
        // Trivial pass-through normalizer; cloned via Arc.
        #[derive(Clone)]
        struct PassNormalizer;
        impl ResultNormalizer for PassNormalizer {
            fn normalize(
                &self,
                _record: &crate::record::ExecutionRecord,
            ) -> Result<NormalizedResult, NormalizationError> {
                Ok(NormalizedResult {
                    evidence_status: EvidenceStatus::Pass,
                    observations: vec![],
                    invalidations: vec![],
                })
            }
        }

        let r = ToolDefinitionRecord::new(
            validate_key(),
            PathBuf::from("/usr/bin/ironmaint-fixture"),
            vec![],
            ExecutionClass::Check,
            ExecutionLimits::default(),
        )
        .with_normalizer(Arc::new(PassNormalizer));

        let n = r.normalizer().expect("normalizer attached");
        assert!(Arc::strong_count(&n) >= 2);
    }
}
