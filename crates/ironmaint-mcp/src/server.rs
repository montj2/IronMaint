//! Streamable HTTP transport for the MCP tool surface
//! (PHASE-0B.md §51, §94).
//!
//! This is the only place in the workspace that knows MCP is
//! spoken over HTTP. It is a library, not a binary: `ironmaintd`
//! owns the process, the socket, and the shutdown policy. Keeping
//! the split that way is what lets this module stay generic over
//! the store and executor — it never names a SQLite backend, so
//! the §98.6 rule that `ironmaint-mcp` may not depend on
//! `ironmaint-store-sqlite` still holds.
//!
//! Two things this module is responsible for, and one it is not:
//!
//! - It advertises exactly the tools `schema::generate_all_schemas`
//!   describes, from that same function. `verify-mcp-schemas`
//!   snapshots the schemas; this serves them. There is no second
//!   list to keep in step, so the advertised surface and the
//!   verified surface cannot drift apart.
//! - It requires a bearer token on **every** request, including
//!   `initialize`. `TokenValidator` existed in 0B with no caller;
//!   this is the caller. The check is axum middleware layered
//!   *outside* rmcp's service, so it runs before rmcp's own
//!   `Host`/`Accept`/`Content-Type` validation. That ordering is
//!   deliberate: rmcp answers a malformed request with 403/406/415
//!   before the body is ever parsed, so an auth check inside the
//!   handler would only ever see well-formed requests and an
//!   unauthenticated caller could map the protocol surface for free.
//! - It is **not** responsible for anything durable. Every call
//!   goes through [`dispatch`], which is the same path the
//!   in-process tests already exercise.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpProtocolError, RoleServer, ServerHandler};

use ironmaint_executor::Executor;
use ironmaint_store::IronMaintStore;

use crate::auth::TokenValidator;
use crate::dispatch::McpRuntime;
use crate::error::McpError;
use crate::schema;

const SERVER_NAME: &str = "ironmaint";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// MCP server handler over an arbitrary store and executor.
///
/// Generic rather than concrete so this crate keeps no persistence
/// dependency of its own: the daemon supplies a `SqliteStore`, the
/// tests supply a `MockStore`, and neither leaks in here.
pub struct IronMaintMcpServer<S: IronMaintStore + ?Sized, E: Executor + ?Sized> {
    runtime: McpRuntime<S, E>,
}

/// The single field is a cloneable handle, so cloning is
/// unconditional — see the matching note on `McpRuntime::clone`.
impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> Clone for IronMaintMcpServer<S, E> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
        }
    }
}

impl<S: IronMaintStore + ?Sized, E: Executor + ?Sized> IronMaintMcpServer<S, E> {
    #[must_use]
    pub fn new(runtime: McpRuntime<S, E>) -> Self {
        Self { runtime }
    }
}

impl<S, E> ServerHandler for IronMaintMcpServer<S, E>
where
    S: IronMaintStore + 'static,
    E: Executor + 'static,
{
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "IronMaint package-maintenance control plane. Create a job with job.create, then \
             follow job.next_actions: each allowed action names the call that performs it. \
             Evidence and gate verdicts are the only trustworthy results — a tool that fails is \
             reported as a Fail verdict, not an error, and an approval always requires a human."
                .to_string(),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpProtocolError> {
        Ok(ListToolsResult::with_all_items(
            schema::generate_all_schemas()
                .keys()
                .filter_map(tool_definition)
                .collect(),
        ))
    }

    /// Advertise a single tool's schema so rmcp can validate
    /// `tools/call` arguments before dispatch (SEP-2243).
    ///
    /// The default returns `None`, which silently disables that
    /// validation. Implementing it means a client that sends the
    /// wrong shape gets a protocol error naming the field, instead
    /// of a runtime error from a deserializer. The schema returned
    /// is byte-identical to the one `tools/list` advertises, so
    /// the check cannot disagree with the advertisement.
    fn get_tool(&self, name: &str) -> Option<Tool> {
        tool_definition(&schema::McpToolName(name.to_string()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpProtocolError> {
        let name = schema::McpToolName(request.name.to_string());
        let arguments = serde_json::Value::Object(request.arguments.unwrap_or_default());

        // A tool that ran and failed is `Ok(CallToolResult::error)`.
        // MCP clients render that content in the conversation, so
        // the caller learns *what went wrong* — a missing job id, a
        // stale revision, a gate that failed. `Err` would be a
        // JSON-RPC protocol error, which clients display opaquely,
        // and would be the wrong level anyway: the request was
        // well-formed and the tool ran.
        match crate::dispatch::dispatch(self.runtime.clone(), &name, arguments).await {
            Ok(output) => Ok(tool_result(output, false)),
            Err(e) => Ok(tool_result(mcp_error_body(&e), true)),
        }
    }
}

/// Build the `Tool` advertisement for one schema-map entry.
fn tool_definition(name: &schema::McpToolName) -> Option<Tool> {
    let schemas = schema::generate_all_schemas();
    let (input, output) = schemas.get(name)?;
    build_tool(&name.0, input, output)
}

fn build_tool(name: &str, input: &serde_json::Value, output: &serde_json::Value) -> Option<Tool> {
    // MCP requires `inputSchema` to be a JSON *object* schema. A
    // schemars value that is not an object would produce a tool no
    // client can call, so refuse to advertise it rather than
    // advertising something broken.
    let input = input.as_object()?.clone();
    let mut tool = Tool::new(
        name.to_string(),
        schema::tool_description(name).unwrap_or(""),
        input,
    );
    // The output schema is the contract for reading a result. It
    // is optional in the protocol, but the agent needs it more
    // than the caller does, and `RunCheckOutput` / `CaptureOutput`
    // are the types that keep the domain result from being guessed
    // at. Advertising it is a strict improvement over omitting it.
    tool.output_schema = output.as_object().cloned().map(Arc::new);
    Some(tool)
}

/// Wrap a JSON value as an MCP tool result.
///
/// `structuredContent` carries the value for clients that read it
/// structurally; the text block carries it for clients that do not.
/// Emitting only the text block would force every agent to parse a
/// string to get an id, so both are sent — on the error path too.
/// A caller that cannot read *why* a tool failed cannot decide
/// whether to fix its input, re-read state, or stop.
fn tool_result(value: serde_json::Value, is_error: bool) -> CallToolResponse {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    let content = vec![ContentBlock::text(text)];
    let mut result = if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    result.structured_content = Some(value);
    result.into()
}

/// The JSON body returned for a failed tool call.
///
/// Shaped like a successful output — `{ "error": { "kind", "message" } }` —
/// so a client parses one shape regardless of outcome. `McpError`
/// serialises as a human string via `Display`; `kind` is the
/// variant name, which is what a client should branch on.
fn mcp_error_body(error: &McpError) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "kind": mcp_error_kind(error),
            "message": error.to_string(),
        }
    })
}

const fn mcp_error_kind(error: &McpError) -> &'static str {
    match error {
        McpError::Auth(_) => "auth",
        McpError::InvalidRequest(_) => "invalid_request",
        McpError::InvalidInput(_) => "invalid_input",
        McpError::Conflict(_) => "conflict",
        McpError::Runtime(_) => "runtime",
        McpError::Internal(_) => "internal",
        McpError::Other(_) => "other",
    }
}

/// Server identity advertised during `initialize`.
#[must_use]
pub fn server_implementation() -> Implementation {
    Implementation::new(SERVER_NAME, SERVER_VERSION)
}

/// Build the rmcp service for a handler.
///
/// Stateless on purpose: `NeverSessionManager` plus
/// `with_legacy_session_mode(false)` means no `Mcp-Session-Id` is
/// issued and no per-client state accumulates. This daemon serves
/// one local agent against a shared store; a session would buy
/// nothing and would leave state to reap on every restart. If some
/// path did try to open a session it would fail loudly rather than
/// quietly creating one, which is the point of choosing
/// `NeverSessionManager` over rmcp's `LocalSessionManager`
/// default.
#[must_use]
pub fn service<S, E>(
    server: IronMaintMcpServer<S, E>,
    config: StreamableHttpServerConfig,
) -> StreamableHttpService<IronMaintMcpServer<S, E>, NeverSessionManager>
where
    S: IronMaintStore + 'static,
    E: Executor + 'static,
{
    StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    )
}

/// Default transport configuration.
///
/// `json_response(true)` because every IronMaint tool is a
/// request/response with no server-initiated stream; SSE framing
/// would add a `data:` prefix and a content-type negotiation
/// failure mode for no benefit. `allowed_hosts` is left at rmcp's
/// loopback default: this daemon binds 127.0.0.1, and the list is
/// the defence against DNS rebinding from a browser on the same
/// machine. A deployment that binds a routable address must widen
/// it deliberately.
#[must_use]
pub fn default_config(
    cancellation: tokio_util::sync::CancellationToken,
) -> StreamableHttpServerConfig {
    StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_cancellation_token(cancellation)
}

/// Bearer-token middleware, applied to every route (PHASE-0B.md
/// §51).
///
/// The response codes are chosen so a client can tell the three
/// failures apart, which is the whole point of authenticating
/// here rather than logging and continuing:
///
/// - **401** — no credential, or a credential that does not match.
///   `WWW-Authenticate: Bearer` tells a client it may retry with
///   one. Deliberately *not* 403: 403 says "you are known and
///   still not allowed", which would be false.
/// - **400** — an `Authorization` header that is not a `Bearer`
///   scheme at all. That is a malformed request, not a failed
///   authentication, and conflating the two teaches clients to
///   retry a bad header forever.
///
/// Note what this does *not* do: it does not distinguish "unknown
/// token" from "wrong token". Both are 401 with the same body. A
/// distinct code for each would tell an attacker which half of
/// their guess was right.
pub async fn require_bearer(
    axum::extract::State(validator): axum::extract::State<TokenValidator>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
    use axum::response::IntoResponse;

    let Some(header) = request.headers().get(AUTHORIZATION) else {
        return (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, "Bearer")],
            "missing Authorization header",
        )
            .into_response();
    };
    let Ok(value) = header.to_str() else {
        return (StatusCode::BAD_REQUEST, "Authorization is not valid ASCII").into_response();
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return (
            StatusCode::BAD_REQUEST,
            "Authorization must use the Bearer scheme",
        )
            .into_response();
    };

    match validator.validate(token.trim()) {
        Ok(()) => next.run(request).await,
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, "Bearer")],
            "token rejected",
        )
            .into_response(),
    }
}

/// Assemble the axum router the daemon serves.
///
/// `layer` rather than `route_layer` on purpose: rmcp's service
/// answers a bad `Host`, `Accept`, or `Content-Type` with 403/406/415
/// *before* it ever looks at a handler, so authentication has to sit
/// outside it. A `route_layer` would still be inside the nesting
/// boundary in some axum versions; a `layer` applied after
/// `nest_service` wraps the whole router unconditionally.
///
/// The path is `/mcp` because that is what `doc/operator/
/// ironclaw-setup.md` and `scripts/ironclaw-e2e.sh` already tell
/// operators to use.
pub fn router<S, E>(
    server: IronMaintMcpServer<S, E>,
    validator: TokenValidator,
    config: StreamableHttpServerConfig,
) -> axum::Router
where
    S: IronMaintStore + 'static,
    E: Executor + 'static,
{
    let mcp = service(server, config);
    axum::Router::new()
        .nest_service("/mcp", mcp)
        .layer(axum::middleware::from_fn_with_state(
            validator,
            require_bearer,
        ))
}
