//! `ToolRegistry` — capability-key → tool definition lookup
//! (PHASE-0B.md §24, §26). The registry owns the registered
//! `ToolDefinition` set; the executor consults it on every run.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use thiserror::Error;

use crate::input::ToolInputMode;
use crate::limits::{ExecutionClass, ExecutionLimits};
use crate::normalizer::ResultNormalizer;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("capability already registered: {0}")]
    AlreadyRegistered(String),
    #[error("capability not found: {0}")]
    NotFound(String),
}

/// Tool definition shape (PHASE-0B.md §27). The runtime constructs
/// `ToolDefinitionRecord` instances and registers them; the executor
/// looks them up by `key` and consults the remaining fields to spawn
/// the subprocess. All accessors are sync — no `async fn` on the trait
/// (constraint: distribution-neutral adapter traits stay sync).
pub trait ToolDefinition: Send + Sync {
    fn key(&self) -> &ToolCapabilityKey;
    fn executable(&self) -> &Path;
    fn fixed_args(&self) -> &[OsString];
    fn class(&self) -> ExecutionClass;
    fn limits(&self) -> ExecutionLimits;
    /// Return a cloneable handle to the registered normalizer. `None`
    /// means the executor will fall back to its default
    /// `FixtureNormalizer` mapping.
    fn normalizer(&self) -> Option<Arc<dyn ResultNormalizer>>;
    /// How the child receives the `ExecutionRequest::input`
    /// payload (PHASE-1.md §7). Defaults to [`ToolInputMode::None`]
    /// so existing implementers — and the six synthetic keys —
    /// keep today's `Stdio::null()` behaviour without needing to
    /// override anything.
    fn input_mode(&self) -> ToolInputMode {
        ToolInputMode::default()
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn ToolDefinition>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("keys", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a tool. Rejects duplicate keys.
    pub fn register(&mut self, tool: Box<dyn ToolDefinition>) -> Result<(), RegistryError> {
        let key = tool.key().as_str().to_string();
        if self.tools.contains_key(&key) {
            return Err(RegistryError::AlreadyRegistered(key));
        }
        self.tools.insert(key, tool);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, key: &ToolCapabilityKey) -> Option<&dyn ToolDefinition> {
        self.tools.get(key.as_str()).map(|b| b.as_ref())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
