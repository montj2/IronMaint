//! `ToolRegistry` — capability-key → tool definition lookup
//! (PHASE-0B.md §24).

use std::collections::BTreeMap;

use ironmaint_adapter_api::ToolCapabilityKey;
use thiserror::Error;

use crate::normalizer::ResultNormalizer;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("capability already registered: {0}")]
    AlreadyRegistered(String),
    #[error("capability not found: {0}")]
    NotFound(String),
}

pub trait ToolDefinition: Send + Sync {
    fn key(&self) -> &ToolCapabilityKey;
    /// Tool-specific normalizer; None means "use the default
    /// exit-code-based classifier".
    fn normalizer(&self) -> Option<Box<dyn ResultNormalizer>>;
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn ToolDefinition>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

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

    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeTool;
    impl ToolDefinition for FakeTool {
        fn key(&self) -> &ToolCapabilityKey {
            static K: std::sync::OnceLock<ToolCapabilityKey> = std::sync::OnceLock::new();
            K.get_or_init(|| ToolCapabilityKey::new("fake.test.dummy").unwrap())
        }
        fn normalizer(&self) -> Option<Box<dyn ResultNormalizer>> {
            None
        }
    }

    #[test]
    fn register_and_get() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(FakeTool)).unwrap();
        let key = ToolCapabilityKey::new("fake.test.dummy").unwrap();
        assert!(r.get(&key).is_some());
        assert!(
            r.get(&ToolCapabilityKey::new("missing.key").unwrap())
                .is_none()
        );
    }

    #[test]
    fn duplicate_register_rejected() {
        let mut r = ToolRegistry::new();
        r.register(Box::new(FakeTool)).unwrap();
        assert!(r.register(Box::new(FakeTool)).is_err());
    }
}
