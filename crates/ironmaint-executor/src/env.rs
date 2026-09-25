//! Sanitised process environment builder (PHASE-0B.md §20).
//!
//! The executor must never pass LLM-controlled env vars to a
//! subprocess. `ProcessEnvironment` is constructed explicitly by
//! the runtime; each entry is allow-listed before being added.
//!
//! This is a deliberately minimal type for 0B.9: the executor
//! skeleton uses it only for fixture subprocesses. The full
//! capability allow-list lands in 0B.12.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessEnvironment {
    vars: BTreeMap<String, String>,
}

impl Default for ProcessEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessEnvironment {
    /// Construct an empty environment with the safety-critical
    /// baseline: `PATH`, `HOME`, `LC_ALL`.
    #[must_use]
    pub fn new() -> Self {
        let mut vars = BTreeMap::new();
        vars.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        vars.insert("HOME".to_string(), "/var/empty".to_string());
        vars.insert("LC_ALL".to_string(), "C.UTF-8".to_string());
        Self { vars }
    }

    /// Add an allow-listed variable. Callers must reject
    /// non-allow-listed keys before calling this.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_baseline() {
        let env = ProcessEnvironment::new();
        assert_eq!(env.get("PATH"), Some("/usr/bin:/bin"));
        assert_eq!(env.get("HOME"), Some("/var/empty"));
        assert_eq!(env.get("LC_ALL"), Some("C.UTF-8"));
        assert!(!env.is_empty());
    }

    #[test]
    fn insert_is_visible() {
        let mut env = ProcessEnvironment::new();
        env.insert("FOO", "bar");
        assert_eq!(env.get("FOO"), Some("bar"));
        assert_eq!(env.len(), 4);
    }
}
