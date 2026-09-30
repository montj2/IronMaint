//! MCP tool dispatcher (§94).
//!
//! Maps a tool name (e.g. `job.create`) plus a JSON value
//! (the tool's input) to a JSON output by delegating to the
//! runtime service. This is the layer the Streamable HTTP
//! transport (commit 15) talks to; it's also what the
//! automated-MCP-client test exercises (§94 exit checkpoint).
//!
//! The dispatcher never reads the store directly. It only
//! speaks `RuntimeCommand`/`RuntimeQuery` to the
//! `RuntimeService`.

use std::sync::Arc;

use ironmaint_executor::Executor;
use ironmaint_runtime::{RuntimeCommand, RuntimeQuery, RuntimeService};
use ironmaint_store::IronMaintStore;
use ironmaint_workspace::{WorkspaceError, WorkspaceErrorKind, WorkspaceManager};

use crate::error::McpError;
use crate::schema::McpToolName;

/// A reference to the runtime service sufficient to dispatch
/// any tool. Holds the runtime behind an `Arc` so the
/// dispatcher is `Clone`-able and cheaply shared across
/// concurrent HTTP handlers.
///
/// The workspace manager is optional: most of the tools are
/// pure runtime calls and need no working tree, and requiring one
/// would make `McpRuntime::new` unusable for them. The three that
/// do need a tree report a typed error when it is absent rather
/// than silently degrading to a stub.
pub struct McpRuntime<S: IronMaintStore + ?Sized, E: Executor + ?Sized> {
    service: Arc<RuntimeService<S, E>>,
    workspace: Option<Arc<WorkspaceManager<Arc<S>>>>,
}

/// Hand-written rather than `#[derive(Clone)]`: every field is an
/// `Arc`, so cloning is unconditional, but the derive would add
/// `S: Clone, E: Clone` bounds the fields do not need — and the
/// transport needs to clone this per request, with a store type
/// that is not `Clone`.
impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> Clone for McpRuntime<S, E> {
    fn clone(&self) -> Self {
        Self {
            service: Arc::clone(&self.service),
            workspace: self.workspace.clone(),
        }
    }
}

impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> McpRuntime<S, E> {
    #[must_use]
    pub fn new(service: Arc<RuntimeService<S, E>>) -> Self {
        Self {
            service,
            workspace: None,
        }
    }

    /// Attach a workspace manager, enabling `workspace.stat`,
    /// `workspace.apply_patch`, and a real `candidate.capture`.
    ///
    /// The manager borrows the *same* store the runtime holds, as
    /// `Arc<S>`. `RuntimeService` keeps its store private and
    /// exposes no accessor (PHASE-0B.md §98.6), and a daemon's
    /// SQLite backend takes an exclusive `fs2` lock on its state
    /// directory, so opening a second connection is not an
    /// option either.
    #[must_use]
    pub fn with_workspace(mut self, workspace: Arc<WorkspaceManager<Arc<S>>>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Borrow the underlying runtime service. Used by tests and
    /// by the Streamable HTTP transport when it needs to do
    /// something the JSON dispatcher can't.
    #[must_use]
    pub fn service(&self) -> &Arc<RuntimeService<S, E>> {
        &self.service
    }

    /// The configured workspace manager, or a typed error naming
    /// the builder call that was missed.
    fn workspace(&self) -> Result<&WorkspaceManager<Arc<S>>, McpError> {
        self.workspace.as_deref().ok_or_else(|| {
            McpError::Internal(
                "this deployment has no workspace manager: the daemon must call \
                 McpRuntime::with_workspace before serving workspace tools"
                    .to_string(),
            )
        })
    }
}

/// Translate a workspace failure into an MCP error, keeping an
/// optimistic-concurrency conflict distinguishable from a genuine
/// runtime failure so a client knows to re-read and retry.
fn workspace_error(e: WorkspaceError) -> McpError {
    match e.kind() {
        WorkspaceErrorKind::Conflict { .. } => McpError::Conflict(e.to_string()),
        other => McpError::Runtime(other.to_string()),
    }
}

/// Dispatch a tool call to the runtime and return its JSON
/// output.
pub async fn dispatch<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: McpRuntime<S, E>,
    tool: &McpToolName,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    match tool.0.as_str() {
        "job.create" => dispatch_job_create(&runtime, input).await,
        "job.get" => dispatch_job_get(&runtime, input).await,
        "job.next_actions" => dispatch_next_actions(&runtime, input).await,
        "job.reconcile" => dispatch_reconcile(&runtime, input).await,
        "job.resume" => dispatch_resume(&runtime, input).await,
        "candidate.capture" => dispatch_capture(&runtime, input).await,
        "check.run" => dispatch_check_run(&runtime, input).await,
        "workspace.apply_patch" => dispatch_apply_patch(&runtime, input).await,
        "workspace.stat" => dispatch_workspace_stat(&runtime, input).await,
        "operation.get" => dispatch_operation_get(&runtime, input).await,
        unknown => Err(McpError::Other(format!("unknown tool: {unknown}"))),
    }
}

async fn dispatch_job_create<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::job::CreateJobInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let result = runtime
        .service
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: input.orchestrator,
            package: input.package,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    // Extract the job_id from the runtime's side-effect: the
    // CreateJob handler writes `format!("job:{id} created ...")`,
    // so the JobId is the first whitespace-delimited token after
    // the `job:` prefix.
    let job_id = result
        .side_effects
        .first()
        .and_then(|s| s.strip_prefix("job:"))
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(ironmaint_core::JobId::from_uuid)
        .ok_or_else(|| McpError::Other("missing job id".into()))?;
    let out = crate::tools::job::CreateJobOutput { job_id };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_job_get<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::job::GetJobInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let query = runtime
        .service
        .handle_query(RuntimeQuery::GetJob {
            job_id: input.job_id,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    let projection = match query {
        ironmaint_runtime::QueryResult::Projection(p) => p,
        _ => return Err(McpError::Other("GetJob returned wrong variant".into())),
    };
    let out = crate::tools::job::GetJobOutput { projection };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_next_actions<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::actions::NextActionsInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let query = runtime
        .service
        .handle_query(RuntimeQuery::ListNextActions {
            job_id: input.job_id,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    let actions = match query {
        ironmaint_runtime::QueryResult::NextActions(a) => a,
        _ => {
            return Err(McpError::Other(
                "ListNextActions returned wrong variant".into(),
            ));
        }
    };
    let out = crate::tools::actions::NextActionsOutput { actions };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_reconcile<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::actions::ReconcileInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let outcome = runtime
        .service
        .reconcile(input.job_id)
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    let out = crate::tools::actions::ReconcileOutput { outcome };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_resume<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::actions::ResumeInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    runtime
        .service
        .handle_command(RuntimeCommand::ResumeJob {
            job_id: input.job_id,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    // Read the resulting state back through the runtime's query
    // path, for the same reason `check.run` does: the command
    // reports what it changed, but the durable answer is a
    // separate read, and parsing the side-effect string here
    // would give this tool a second, subtly different account of
    // where the job ended up.
    let query = runtime
        .service
        .handle_query(RuntimeQuery::GetJob {
            job_id: input.job_id,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    // `QueryResult::Projection` carries the projection as
    // `serde_json::Value` (it is the MCP wire form), so recover
    // the typed state from it rather than reaching into the
    // object's fields.
    let value = match query {
        ironmaint_runtime::QueryResult::Projection(p) => p,
        _ => return Err(McpError::Other("GetJob returned wrong variant".into())),
    };
    let projection: ironmaint_core::JobProjection =
        serde_json::from_value(value).map_err(|e| McpError::Internal(e.to_string()))?;
    let out = crate::tools::actions::ResumeOutput {
        resumed_to: projection.state,
    };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_capture<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::candidate::CaptureInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let workspace = runtime.workspace()?;
    let id = workspace
        .ensure_workspace(input.job_id)
        .await
        .map_err(workspace_error)?;

    // `capture_candidate` reads the real HEAD commit and the working
    // tree's tree OID, so the fingerprint actually reflects source
    // state. The 0B.6 stub hardcoded `"0".repeat(40)` for both,
    // which made every capture of a given package collide on one
    // fingerprint — and since `handle_capture_candidate` dedupes by
    // fingerprint, a genuine source update would have been silently
    // dropped as a duplicate.
    let candidate = workspace
        .capture_candidate(id, input.job_id, input.package, &input.repository_url)
        .await
        .map_err(workspace_error)?;
    let fingerprint = candidate.fingerprint().clone();

    // The workspace manager already persisted the candidate row in
    // order to mint its `CandidateId` for the maintenance commit.
    // The runtime's handler is idempotent — it looks the candidate
    // up by fingerprint and reuses the existing id — so this
    // records the audit envelope in the job's event log without
    // writing a second row. `candidate_capture_is_idempotent`
    // pins that.
    //
    // The clone is for the workspace-activation step below, which
    // needs the candidate after the command has taken it. A
    // `SourceCandidate` is a handful of identifiers, so paying for
    // it beats reading the row back out of the store to recover
    // what we were just holding.
    let to_activate = candidate.clone();
    let result = runtime
        .service
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id: input.job_id,
            candidate,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;

    // §43 closes `candidate.capture` with "mark workspace clean",
    // and until this line nothing did. `WorkspaceManager::activate_candidate`
    // is the only writer that clears `dirty`, and the only caller it
    // had was a workspace test — so over the tool surface a job could
    // be patched exactly once. `apply_patch` refuses a dirty tree,
    // §101 needs three patches, and the second one failed with
    // "workspace is dirty" naming a cause the agent had no way to
    // clear.
    //
    // This runs *after* the runtime's capture, not before: the
    // workspace must not be marked clean for a candidate the runtime
    // refused.
    let mut notes = result.side_effects;
    match workspace.activate_candidate(id, &to_activate).await {
        Ok(new_revision) => notes.push(format!(
            "workspace marked clean at revision {new_revision}; the next \
             workspace.apply_patch is accepted"
        )),
        Err(e) => notes.push(format!(
            "captured {fingerprint} but could not mark the workspace clean: \
             {e}. The job is at the captured source; a further \
             workspace.apply_patch will be refused until this succeeds."
        )),
    }

    // The side effects are returned rather than logged: they are the
    // only place an agent can learn that the candidate was not
    // activated, or that no adapter claims its family, or that the
    // adapter it did find plans no gates. See `CaptureOutput::notes`.
    let out = crate::tools::candidate::CaptureOutput { fingerprint, notes };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_check_run<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::check::RunCheckInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    runtime
        .service
        .handle_command(RuntimeCommand::RunCheck {
            check_id: input.check_id,
            retry_class: input
                .retry_class
                .unwrap_or(ironmaint_executor::RetryClass::Safe),
            job_id: input.job_id,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    // Read the outcome back through the runtime's own query path
    // rather than parsing the command's stringly-typed
    // `side_effects`. The command deliberately reports only what it
    // changed; the durable verdict is a separate read, and reusing
    // the query keeps `check.run` from growing a second, subtly
    // different answer.
    let query = runtime
        .service
        .handle_query(RuntimeQuery::GetCheckOutcome {
            check_id: input.check_id,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    let ironmaint_runtime::QueryResult::CheckOutcome(outcome) = query else {
        return Err(McpError::Other(
            "GetCheckOutcome returned wrong variant".into(),
        ));
    };
    let out = crate::tools::check::RunCheckOutput::from(outcome);
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_apply_patch<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::workspace::ApplyPatchInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let workspace = runtime.workspace()?;
    let id = workspace
        .ensure_workspace(input.job_id)
        .await
        .map_err(workspace_error)?;
    // Real CAS: a stale `expected_revision` surfaces as
    // `WorkspaceErrorKind::Conflict` and is mapped to
    // `McpError::Conflict`, so the caller can tell "re-read and
    // retry" from "this will never work". The 0B.6 stub echoed
    // `expected_revision` back unchanged, which would have made
    // the documented retry contract unimplementable.
    let new_revision = workspace
        .apply_patch(id, input.expected_revision, &input.patch)
        .await
        .map_err(workspace_error)?;
    let out = crate::tools::workspace::ApplyPatchOutput { new_revision };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_workspace_stat<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::workspace::StatInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let workspace = runtime.workspace()?;
    let id = workspace
        .ensure_workspace(input.job_id)
        .await
        .map_err(workspace_error)?;
    let revision = workspace
        .current_revision(id)
        .await
        .map_err(workspace_error)?;
    let out = crate::tools::workspace::StatOutput { revision };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_operation_get<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::operation::GetInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    // Routed through the runtime, not a store handle the MCP layer
    // holds (§98.6). The 0B.6 stub returned a sentinel `proposed`
    // record for *any* id, so `operation.get` never failed and
    // never reported a real operation — a caller could not tell a
    // real `Authorized` push from the placeholder.
    let query = runtime
        .service
        .handle_query(RuntimeQuery::GetOperation {
            operation_id: input.operation_id,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    let ironmaint_runtime::QueryResult::Operation(op) = query else {
        return Err(McpError::Other(
            "GetOperation returned wrong variant".into(),
        ));
    };
    let out = crate::tools::operation::GetOutput { operation: op };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}
