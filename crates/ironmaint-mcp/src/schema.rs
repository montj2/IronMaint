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
        McpToolName("job.reconcile".to_string()),
        (
            serde_json::to_value(schema_for!(tools::actions::ReconcileInput))
                .unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::actions::ReconcileOutput))
                .unwrap_or(Value::Null),
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

/// One-line descriptions advertised by `tools/list`.
///
/// Kept here, next to `generate_all_schemas`, rather than in the
/// transport, because a tool with no description is a tool an
/// agent cannot choose between — and a description attached to a
/// name the schema map does not contain is worse than none. The
/// `descriptions_cover_every_tool` test below is what makes the
/// second failure impossible rather than merely unlikely.
#[must_use]
pub fn tool_description(name: &str) -> Option<&'static str> {
    Some(match name {
        "job.create" => "Start a package-maintenance job for one package in one distribution.",
        "job.get" => "Read a job's current projection: state, active candidate, version.",
        "job.next_actions" => {
            "List the moves the orchestrator may take now, and the blockers on the rest."
        }
        "job.reconcile" => {
            "Walk the state machine's rule table for one job and report where it stops."
        }
        "candidate.capture" => {
            "Capture the workspace's current source state as a SourceCandidate, returning \
             its fingerprint."
        }
        "check.run" => {
            "Run one materialised check by check_id and return the evidence and gate verdict."
        }
        "workspace.apply_patch" => {
            "Apply a patch to the job's working tree, bumping the revision under \
             compare-and-swap."
        }
        "workspace.stat" => "Read the job's working-tree revision.",
        "operation.get" => "Read one privileged operation's kind and authorization state.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_cover_every_tool() {
        for name in tool_names() {
            assert!(
                tool_description(&name.0).is_some(),
                "{} is advertised with no description",
                name.0
            );
        }
    }

    #[test]
    fn no_description_is_invented_for_an_unknown_tool() {
        assert!(tool_description("job.teleport").is_none());
    }

    #[test]
    fn the_surface_is_nine_tools() {
        // §94 enumerates nine. A tenth would be a scope change and
        // a client-compatibility question, so it should be a
        // deliberate edit to this assertion, not a surprise.
        assert_eq!(tool_names().len(), 9);
    }
}
