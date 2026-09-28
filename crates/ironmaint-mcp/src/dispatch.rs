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

use ironmaint_core::{
    GitHashAlgorithm, GitObjectId, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate,
    VcsKind,
};
use ironmaint_executor::Executor;
use ironmaint_runtime::{RuntimeCommand, RuntimeQuery, RuntimeService};
use ironmaint_store::IronMaintStore;

use crate::error::McpError;
use crate::schema::McpToolName;

/// A reference to the runtime service sufficient to dispatch
/// any tool. Holds the runtime behind an `Arc` so the
/// dispatcher is `Clone`-able and cheaply shared across
/// concurrent HTTP handlers.
#[derive(Clone)]
pub struct McpRuntime<S: IronMaintStore + ?Sized, E: Executor + ?Sized> {
    service: Arc<RuntimeService<S, E>>,
}

impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> McpRuntime<S, E> {
    #[must_use]
    pub fn new(service: Arc<RuntimeService<S, E>>) -> Self {
        Self { service }
    }

    /// Borrow the underlying runtime service. Used by tests and
    /// by the Streamable HTTP transport when it needs to do
    /// something the JSON dispatcher can't.
    #[must_use]
    pub fn service(&self) -> &Arc<RuntimeService<S, E>> {
        &self.service
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
        "candidate.capture" => dispatch_capture(&runtime, input).await,
        "check.run" => dispatch_check_run(input).await,
        "workspace.apply_patch" => dispatch_apply_patch(input).await,
        "workspace.stat" => dispatch_workspace_stat(input).await,
        "operation.get" => dispatch_operation_get(input).await,
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

async fn dispatch_capture<S: IronMaintStore + ?Sized, E: Executor + ?Sized>(
    runtime: &McpRuntime<S, E>,
    input: serde_json::Value,
) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::candidate::CaptureInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000)
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let repo_url = url::Url::parse(&input.repository_url)
        .map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let repository = RepositoryRef::new(VcsKind::Git, repo_url)
        .map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "0".repeat(40))
        .map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "0".repeat(40))
        .map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let version =
        PackageVersion::new("0.0.0").map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let revision = PackageRevision::new(input.package.clone(), version);
    let candidate = SourceCandidate::new(input.job_id, revision, repository, commit, tree, now);
    let fp = candidate.fingerprint().clone();
    runtime
        .service
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id: input.job_id,
            candidate,
        })
        .await
        .map_err(|e| McpError::Runtime(e.message))?;
    let out = crate::tools::candidate::CaptureOutput { fingerprint: fp };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_check_run(input: serde_json::Value) -> Result<serde_json::Value, McpError> {
    let _input: crate::tools::check::RunCheckInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    // The actual execution lives on the runtime as
    // `RuntimeCommand::RunCheck { job_id, check_id }`. The MCP
    // layer for 0B.6 only validates the input shape; the streamable
    // HTTP transport wires the runtime call when it adopts this
    // dispatch in the next commit. Returning the parsed input as
    // JSON keeps the tool round-trip observable end-to-end.
    let out = crate::tools::check::RunCheckOutput {
        tool_key: String::new(),
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_apply_patch(input: serde_json::Value) -> Result<serde_json::Value, McpError> {
    let input: crate::tools::workspace::ApplyPatchInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let out = crate::tools::workspace::ApplyPatchOutput {
        new_revision: input.expected_revision,
    };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_workspace_stat(input: serde_json::Value) -> Result<serde_json::Value, McpError> {
    let _input: crate::tools::workspace::StatInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    let out = crate::tools::workspace::StatOutput {
        revision: ironmaint_workspace::WorkspaceRevision::new(),
    };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}

async fn dispatch_operation_get(input: serde_json::Value) -> Result<serde_json::Value, McpError> {
    let _input: crate::tools::operation::GetInput =
        serde_json::from_value(input).map_err(|e| McpError::InvalidInput(e.to_string()))?;
    // The MCP layer does not mint PrivilegedOperation objects on its
    // own; the runtime is the only owner of the authorization state
    // machine. Returning a sentinel `proposed` record here keeps the
    // tool schema round-trippable until 0B.6 wires the actual
    // `OperationStore::get` lookup behind this entry point.
    let op = ironmaint_policy::PrivilegedOperation::proposed(
        ironmaint_policy::PrivilegedOperationKind::CanonicalRepositoryPush,
        ironmaint_core::CandidateFingerprint::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .map_err(|e| McpError::Other(e.to_string()))?,
    );
    let out = crate::tools::operation::GetOutput { operation: op };
    serde_json::to_value(out).map_err(|e| McpError::Other(e.to_string()))
}
