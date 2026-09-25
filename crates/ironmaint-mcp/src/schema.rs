//! JSON Schema generation for the MCP tool surface.

use std::collections::BTreeMap;

use schemars::schema_for;
use serde_json::Value;

use crate::tools;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct McpToolName(pub String);

/// Generate JSON Schemas for every tool's input/output. The
/// returned map is keyed by `McpToolName` and contains
/// `{ "input": Schema, "output": Schema }`.
#[must_use]
pub fn generate_all_schemas() -> BTreeMap<McpToolName, (Value, Value)> {
    let mut out = BTreeMap::new();
    out.insert(
        McpToolName("job.create".to_string()),
        (
            serde_json::to_value(schema_for!(tools::job::CreateJobInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::job::CreateJobOutput)).unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("job.get".to_string()),
        (
            serde_json::to_value(schema_for!(tools::job::GetJobInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::job::GetJobOutput)).unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("job.next_actions".to_string()),
        (
            serde_json::to_value(schema_for!(tools::actions::NextActionsInput))
                .unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::actions::NextActionsOutput))
                .unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("candidate.capture".to_string()),
        (
            serde_json::to_value(schema_for!(tools::candidate::CaptureInput))
                .unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::candidate::CaptureOutput))
                .unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("check.run".to_string()),
        (
            serde_json::to_value(schema_for!(tools::check::RunCheckInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::check::RunCheckOutput)).unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("workspace.apply_patch".to_string()),
        (
            serde_json::to_value(schema_for!(tools::workspace::ApplyPatchInput))
                .unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::workspace::ApplyPatchOutput))
                .unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("workspace.stat".to_string()),
        (
            serde_json::to_value(schema_for!(tools::workspace::StatInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::workspace::StatOutput)).unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("operation.get".to_string()),
        (
            serde_json::to_value(schema_for!(tools::operation::GetInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::operation::GetOutput)).unwrap_or(Value::Null),
        ),
    );
    out
}

#[must_use]
pub fn tool_names() -> Vec<McpToolName> {
    generate_all_schemas().keys().cloned().collect()
}
