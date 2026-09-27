//! Execution record — the durable, JSON-serialised result of a
//! tool invocation.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::json_schema_impls::Rfc3339DateTime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::retry::RetryClass;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionRecord {
    pub tool_key: ToolCapabilityKey,
    pub retry_class: RetryClass,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "Rfc3339DateTime")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "Rfc3339DateTime")]
    pub finished_at: OffsetDateTime,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Whether the executor's retry budget was exhausted.
    #[serde(default)]
    pub retries_exhausted: bool,
    /// `true` iff either stdout or stderr was bounded by the
    /// configured `ExecutionLimits.stdout_max_bytes` /
    /// `stderr_max_bytes` cap (PHASE-0B.md §15). Truncation is
    /// orthogonal to outcome: a truncated `PASS` is still a `PASS`.
    /// Default `false` so wire-format payloads from before 0B.4
    /// parse unchanged.
    #[serde(default)]
    pub truncated: bool,
}

impl ExecutionRecord {
    #[must_use]
    pub fn duration_ms(&self) -> i64 {
        let ms = (self.finished_at - self.started_at).whole_milliseconds();
        if ms <= 0 {
            0
        } else {
            ms.min(i64::MAX as i128) as i64
        }
    }

    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.exit_code == 0
    }

    /// Builder-style setter for [`Self::truncated`].
    #[must_use]
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample(exit_code: i32) -> ExecutionRecord {
        ExecutionRecord {
            tool_key: ToolCapabilityKey::new("synthetic.build.validate").unwrap(),
            retry_class: RetryClass::Safe,
            started_at: datetime!(2026-01-01 00:00:00 UTC),
            finished_at: datetime!(2026-01-01 00:00:01 UTC),
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
            retries_exhausted: false,
            truncated: false,
        }
    }

    #[test]
    fn execution_record_default_truncated_is_false() {
        assert!(!sample(0).truncated);
    }

    #[test]
    fn with_truncated_sets_flag() {
        let r = sample(0).with_truncated(true);
        assert!(r.truncated);
    }

    #[test]
    fn execution_record_truncated_round_trips_via_json() {
        let r = sample(0).with_truncated(true);
        let json = serde_json::to_string(&r).unwrap();
        let back: ExecutionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn truncated_defaults_to_false_on_missing_field() {
        // Older wire payloads omit `truncated`; serde(default) must
        // restore it to `false`.
        let json = serde_json::json!({
            "tool_key": "synthetic.build.validate",
            "retry_class": "safe",
            "started_at": "2026-01-01T00:00:00Z",
            "finished_at": "2026-01-01T00:00:01Z",
            "exit_code": 0,
            "stdout": "",
            "stderr": "",
            "retries_exhausted": false,
        })
        .to_string();
        let r: ExecutionRecord = serde_json::from_str(&json).unwrap();
        assert!(!r.truncated);
    }
}
