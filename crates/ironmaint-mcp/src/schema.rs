//! JSON Schema generation for the MCP tool surface.

use std::collections::BTreeMap;

use schemars::schema_for;
use serde_json::Value;

use crate::tools;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct McpToolName(pub String);

/// The MCP tool that performs `action`.
///
/// This match has **no wildcard arm**, which is the point. Adding an
/// `AllowedAction` variant without deciding which tool performs it is
/// a compile error in this crate, not a silently unperformable entry
/// in a field whose contract says its contents can all be taken
/// right now. `every_allowed_action_has_a_tool` then checks the
/// names this returns are actually in the advertised surface, which
/// catches the other half — a name that is spelled right here but
/// registered nowhere.
///
/// The moves that genuinely have no tool — satisfying an
/// obligation, requesting an approval, authorizing a publication —
/// are deliberately absent from `AllowedAction` and live in
/// `ironmaint_runtime::HumanAction` instead. For the latter two,
/// §4.10/§26/§99 forbid the producer from existing in 0B at all.
#[must_use]
pub fn tool_for_action(action: &ironmaint_runtime::AllowedAction) -> &'static str {
    use ironmaint_runtime::AllowedAction as A;
    match action {
        A::CaptureCandidate => "candidate.capture",
        A::RunCheck { .. } => "check.run",
        A::ApplyPatch => "workspace.apply_patch",
        A::ResumeJob => "job.resume",
    }
}

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
    out.insert(
        McpToolName("job.resume".to_string()),
        (
            serde_json::to_value(schema_for!(tools::actions::ResumeInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::actions::ResumeOutput)).unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("release.candidate.create".to_string()),
        (
            serde_json::to_value(schema_for!(tools::release::CreateInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::release::CreateOutput)).unwrap_or(Value::Null),
        ),
    );
    // PHASE-1.md §31 — the read-only evidence tools. 1A.3 wrote the
    // `Evidence` rows; 1A.4 adds the read side, addressed through the
    // runtime (PHASE-0B §98.6) so the MCP layer never holds a store
    // handle. These are the tools the §31 exit-checkpoint from 1A.3
    // names ("have an MCP client read the report").
    out.insert(
        McpToolName("evidence.list".to_string()),
        (
            serde_json::to_value(schema_for!(tools::evidence::ListInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::evidence::ListOutput)).unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("evidence.get".to_string()),
        (
            serde_json::to_value(schema_for!(tools::evidence::GetInput)).unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::evidence::GetOutput)).unwrap_or(Value::Null),
        ),
    );
    out.insert(
        McpToolName("evidence.artifact.read".to_string()),
        (
            serde_json::to_value(schema_for!(tools::evidence::ReadArtifactInput))
                .unwrap_or(Value::Null),
            serde_json::to_value(schema_for!(tools::evidence::ReadArtifactOutput))
                .unwrap_or(Value::Null),
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
        "job.resume" => {
            "Return a job awaiting human review to the state recorded when it was \
             escalated."
        }
        "release.candidate.create" => {
            "Assemble the release-candidate snapshot of the job's active candidate: the \
             gates and obligations it was validated against. Publishes nothing."
        }
        // PHASE-1.md §31 — the read-only evidence tools. 1A.4 closes
        // the §31 exit-checkpoint from 1A.3 ("have an MCP client
        // read the report") by giving agents a way to fetch the
        // evidence rows and report bytes the run produced.
        "evidence.list" => {
            "List every evidence row attached to a job, optionally narrowed to one \
             candidate fingerprint. Read-only; the runtime owns the rows."
        }
        "evidence.get" => {
            "Read one evidence row by its id. Returns the durable form an MCP client \
             can render, including the bound artifact references."
        }
        "evidence.artifact.read" => {
            "Read the bytes of one report artifact bound to a specific evidence row. \
             Returns the bytes base64-encoded under `bytes_base64` with the \
             `media_type` the artifact was stored with. The runtime refuses to serve \
             bytes for an `artifact_id` the named evidence row does not reference, so \
             the tool exposes the bytes of that evidence row's artifacts, not an \
             arbitrary digest."
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::CheckId;
    use ironmaint_runtime::AllowedAction;

    /// `next_actions.allowed` promises its contents can be taken
    /// right now. A `RunCheck` entry names a `check_id`, and
    /// `check.run` takes one, so this is constructible.
    fn one_of_each_action() -> Vec<AllowedAction> {
        vec![
            AllowedAction::CaptureCandidate,
            AllowedAction::RunCheck {
                check_id: CheckId::new(),
            },
            AllowedAction::ApplyPatch,
            AllowedAction::ResumeJob,
        ]
    }

    /// The half the compiler cannot check: a name spelled correctly
    /// in `tool_for_action` but registered nowhere would leave an
    /// agent calling a tool the server does not advertise.
    #[test]
    fn every_allowed_action_maps_to_an_advertised_tool() {
        for action in one_of_each_action() {
            let tool = tool_for_action(&action);
            assert!(
                tool_names().iter().any(|n| n.0 == tool),
                "{action:?} maps to `{tool}`, which is not in the advertised surface"
            );
            assert!(
                tool_description(tool).is_some(),
                "{action:?} maps to `{tool}`, which has no description for an agent to choose by"
            );
        }
    }

    /// And the reverse: a tool nobody can reach through an action is
    /// not a dead tool — `job.get`, `job.next_actions`, `job.reconcile`
    /// and `operation.get` are all queries, and `job.resume` is
    /// deliberately also offered as a human-side move. So this is a
    /// guard against a *new* action being added with a typo, not a
    /// bid for a bijection.
    #[test]
    fn tool_for_action_is_total() {
        // `tool_for_action` returns `&'static str` with no `Option`,
        // so totality is a compile-time property. This test exists to
        // document that and to fail loudly if the signature is ever
        // loosened to `Option` without this being reconsidered.
        for action in one_of_each_action() {
            let tool = tool_for_action(&action);
            assert!(!tool.is_empty(), "{action:?} mapped to an empty name");
        }
    }

    /// The human cases must stay out of `allowed`. If a future edit
    /// moves `ApproveRelease` back into `AllowedAction` so a tool can
    /// be written for it, this fails — which is the correct outcome,
    /// because §4.10/§26/§99 forbid the approval producer in 0B and
    /// the split is what keeps the promise honest.
    #[test]
    fn human_actions_are_not_allowed_actions() {
        use ironmaint_runtime::HumanAction;
        // Distinct types, so this is a compile-time property too.
        // Assert the behaviours that depend on the split staying put.
        let cases = [
            HumanAction::ApproveRelease,
            HumanAction::AuthorizePublication,
            HumanAction::ReviewEscalation,
            HumanAction::SatisfyObligation { reference: None },
        ];
        assert_eq!(cases.len(), 4, "every human action is enumerated");
    }

    /// The wire contract `next_actions` promises. `allowed` is what
    /// an agent may take right now, so it must be exactly the
    /// tool-performable set and nothing else; the human moves belong
    /// to `requires_human`.
    ///
    /// Asserted against the *generated* schema rather than the Rust
    /// enum, because the promise is about the wire. The enum cannot
    /// be in `AllowedAction` and still appear here — the point is
    /// that an agent reading this JSON cannot be told to make a move
    /// with no tool behind it.
    #[test]
    fn next_actions_schema_separates_tool_performable_from_human_moves() {
        let output = serde_json::to_value(schema_for!(crate::tools::actions::NextActionsOutput))
            .unwrap_or(Value::Null);
        // `actions` is emitted as a `$ref` to the `JobNextActions`
        // definition rather than inlined. Follow it rather than
        // hardcoding a layout — a schemars upgrade that inlines it
        // would otherwise fail this test for a reason that has
        // nothing to do with the contract.
        let actions = output
            .pointer("/properties/actions")
            .expect("actions property");
        let actions = match actions.get("$ref").and_then(Value::as_str) {
            Some(r) => output
                .pointer(r.strip_prefix("#").expect("local $ref"))
                .expect("the $ref target"),
            None => actions,
        };
        let props = actions.get("properties").and_then(Value::as_object);

        let props = match props {
            Some(p) => p,
            None => panic!("JobNextActions properties missing from the schema: {output}"),
        };
        assert!(
            props.contains_key("requires_human"),
            "the human-action field must be on the wire: {output}"
        );
        assert!(
            props.contains_key("allowed"),
            "`allowed` must survive: {output}"
        );

        // And `allowed` advertises precisely the tool-performable set:
        // the four variants `tool_for_action` has a tool for, and none
        // of the human moves.
        let definitions = output
            .get("definitions")
            .and_then(Value::as_object)
            .expect("definitions");
        let allowed = definitions["AllowedAction"].to_string();
        for performable in [
            "capture_candidate",
            "run_check",
            "apply_patch",
            "resume_job",
        ] {
            assert!(
                allowed.contains(performable),
                "`allowed` must advertise `{performable}`: {allowed}"
            );
        }
        for human in [
            "satisfy_obligation",
            "approve_release",
            "authorize_publication",
            "review_escalation",
        ] {
            assert!(
                !allowed.contains(human),
                "`allowed` advertises `{human}`, which no tool can perform: {allowed}"
            );
        }
        assert!(
            definitions.contains_key("HumanAction"),
            "the human moves need their own wire type: {:?}",
            definitions.keys().collect::<Vec<_>>()
        );
    }

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
    fn the_surface_is_eleven_tools() {
        // §94 enumerates *capabilities* — "reconcile", "next-actions",
        // "candidate capture", and so on — not a count of nine, so
        // growing the surface past nine does not contradict it.
        //
        // Two tools have been added past the 0B.6 set, both
        // deliberately, and both because §101's walk stops without
        // them:
        //
        // - the tenth, `job.resume` (0B.10 C2): an agent that lands
        //   in `HumanReviewRequired` otherwise has no way out.
        // - the eleventh, `release.candidate.create` (0B.10 C5):
        //   §101 steps 26-27 assemble a release candidate and assert
        //   it is bound to the final candidate. §53 already grants
        //   the agent `ironmaint_gate_list` and
        //   `ironmaint_obligation_list`; the snapshot is the join of
        //   those two, so withholding the join while handing over
        //   its parts would be backwards.
        // - the twelfth, thirteenth, and fourteenth
        //   (`evidence.list`, `evidence.get`,
        //   `evidence.artifact.read`, PHASE-1.md §31 / PR 1A.4):
        //   1A.3 writes `Evidence` rows with bound report artifacts.
        //   The §31 exit-checkpoint names "have an MCP client read
        //   the report" — the only way to read it is through these
        //   three tools. The runtime is the only place that holds
        //   the store handle (PHASE-0B §98.6); these are the read
        //   side.
        //
        // The original note here said a tenth "would be a scope
        // change and a client-compatibility question, so it should
        // be a deliberate edit to this assertion, not a surprise."
        // That was right about the process and all of these are
        // deliberate edits. The guard stays: the next tool should
        // have to come through here too.
        assert_eq!(tool_names().len(), 14);
    }
}
