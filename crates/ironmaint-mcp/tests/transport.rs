//! End-to-end proof that an MCP server exists and is reachable.
//!
//! PHASE-0B-COMPLETION.md §102 item 26 ("IronClaw can call the MCP
//! server") was recorded as *unverified* while in fact being false:
//! `ironmaint-mcp` was a library with no socket, and `ironmaintd`
//! carried a comment marking the hookup point that never landed.
//! Nothing in the workspace could be reached over HTTP, so nothing
//! about the transport could be tested.
//!
//! These tests bind a real listener on an ephemeral port and speak
//! real HTTP to it. An in-process call to `dispatch()` would prove
//! nothing here — the claim is about the wire.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_mcp::schema::tool_names;
use ironmaint_mcp::{
    AuthToken, IronMaintMcpServer, McpRuntime, TokenValidator, default_config, router,
};
use ironmaint_runtime::{RuntimeService, SystemClock};
use ironmaint_store::mock::MockStore;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "ironmaint-fixture-token-0123456789";

/// rmcp requires `Accept` to list both media types on a POST; a
/// request offering only one is rejected 406 before dispatch.
/// Getting this wrong makes every other assertion here fail with a
/// status code that says nothing about the real problem, so it is
/// a named constant rather than a string repeated inline.
const ACCEPT: &str = "application/json, text/event-stream";

struct Harness {
    base: String,
    client: reqwest::Client,
    shutdown: CancellationToken,
}

impl Harness {
    async fn start() -> Self {
        let store = Arc::new(MockStore::new());
        let clock: Arc<dyn ironmaint_runtime::Clock> = Arc::new(SystemClock);
        let service = Arc::new(RuntimeService::new(
            Arc::clone(&store),
            clock,
            Arc::new(NullExecutor),
            Arc::new(ToolRegistry::new()),
        ));
        let shutdown = CancellationToken::new();
        let app = router(
            IronMaintMcpServer::new(McpRuntime::new(service)),
            TokenValidator::new(AuthToken::new(TOKEN)),
            default_config(shutdown.clone()),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        let token = shutdown.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move { token.cancelled_owned().await })
                .await;
        });

        Self {
            base: format!("http://{addr}/mcp"),
            client: reqwest::Client::new(),
            shutdown,
        }
    }

    fn stop(&self) {
        self.shutdown.cancel();
    }

    /// POST a JSON-RPC body, optionally authenticated.
    async fn post(&self, body: Value, token: Option<&str>) -> reqwest::Response {
        let mut request = self
            .client
            .post(&self.base)
            .header("Content-Type", "application/json")
            .header("Accept", ACCEPT)
            .header("Host", "127.0.0.1");
        if let Some(t) = token {
            request = request.header("Authorization", format!("Bearer {t}"));
        }
        request.json(&body).send().await.expect("send")
    }

    async fn initialize(&self, token: Option<&str>) -> reqwest::Response {
        self.post(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "ironmaint-test", "version": "1.0"},
                },
            }),
            token,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// The socket exists
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_authenticated_initialize_is_answered_over_http() {
    // The load-bearing test. Before 0B.9 there was no way to write
    // it: no listener, no router, no server handler.
    let h = Harness::start().await;
    let response = h.initialize(Some(TOKEN)).await;
    assert_eq!(response.status(), 200, "initialize must be served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    assert!(
        body["result"]["capabilities"]["tools"].is_object(),
        "tools capability must be advertised: {body}"
    );
    h.stop();
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[tokio::test]
async fn initialize_without_a_token_is_401() {
    // §51 requires the token on *every* call, `initialize`
    // included. Skipping the handshake would let an unauthenticated
    // caller learn the server's name, version, and capability set.
    let h = Harness::start().await;
    let response = h.initialize(None).await;
    assert_eq!(response.status(), 401);
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer"),
        "401 must advertise the scheme so a client can retry"
    );
    h.stop();
}

#[tokio::test]
async fn initialize_with_the_wrong_token_is_401() {
    let h = Harness::start().await;
    assert_eq!(h.initialize(Some("not-the-token")).await.status(), 401);
    h.stop();
}

#[tokio::test]
async fn a_token_that_is_a_prefix_of_the_real_one_is_401() {
    // The length check in `TokenValidator` is what makes this fail;
    // without it, a prefix would pass a naive prefix-match. Worth
    // pinning over the wire because the middleware is the first
    // code path to call the validator at all.
    let h = Harness::start().await;
    let prefix = &TOKEN[..TOKEN.len() - 1];
    assert_eq!(h.initialize(Some(prefix)).await.status(), 401);
    h.stop();
}

#[tokio::test]
async fn a_token_with_the_right_prefix_but_wrong_tail_is_401() {
    let h = Harness::start().await;
    let wrong = format!("{}X", &TOKEN[..TOKEN.len() - 1]);
    assert_eq!(h.initialize(Some(&wrong)).await.status(), 401);
    h.stop();
}

#[tokio::test]
async fn a_non_bearer_authorization_header_is_400_not_401() {
    // A `Basic` header is a malformed request, not a failed
    // authentication. Reporting 401 would tell the client to retry
    // with credentials when the real problem is its header scheme.
    let h = Harness::start().await;
    let response = h
        .post(
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            None,
        )
        .await;
    assert_eq!(response.status(), 401, "no header at all is 401");

    let response = h
        .client
        .post(&h.base)
        .header("Content-Type", "application/json")
        .header("Accept", ACCEPT)
        .header("Authorization", format!("Basic {TOKEN}"))
        .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), 400, "wrong scheme is 400");
    h.stop();
}

#[tokio::test]
async fn auth_is_enforced_before_rmcp_validates_the_request_shape() {
    // Ordering test. rmcp answers a request with no `Accept` header
    // with 406, and one with a bad `Host` with 403. If auth ran
    // inside the handler those codes would reach an unauthenticated
    // caller, who could then map the protocol surface for free —
    // and, worse, a 401 here would mean the middleware was *not*
    // outside rmcp. An unauthenticated request with three protocol
    // violations must still be a 401.
    let h = Harness::start().await;
    let response = h
        .client
        .post(&h.base)
        .header("Content-Type", "application/json")
        // no Accept, no Authorization
        .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .send()
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        401,
        "auth must precede rmcp's own 403/406/415 checks"
    );
    h.stop();
}

// ---------------------------------------------------------------------------
// tools/list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_list_advertises_exactly_the_ten_known_tools() {
    // The advertised surface is generated from the same function
    // `verify-mcp-schemas` snapshots, so this cannot drift from the
    // verified schemas. If it ever does, this test names the
    // mismatch rather than the snapshot check.
    let h = Harness::start().await;
    let response = h
        .post(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {},
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("json");

    let mut advertised: Vec<String> = body["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("tool name").to_string())
        .collect();
    let mut expected: Vec<String> = tool_names().into_iter().map(|n| n.0).collect();
    advertised.sort();
    expected.sort();
    assert_eq!(advertised, expected);
    assert_eq!(advertised.len(), 10);
    h.stop();
}

#[tokio::test]
async fn every_advertised_tool_carries_an_input_schema_and_a_description() {
    // A tool with no `inputSchema` cannot be called by a conforming
    // client, and one with no description is a tool an agent cannot
    // choose between.
    let h = Harness::start().await;
    let body: Value = h
        .post(
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
            Some(TOKEN),
        )
        .await
        .json()
        .await
        .expect("json");
    for tool in body["result"]["tools"].as_array().expect("tools") {
        let name = tool["name"].as_str().expect("name");
        assert!(
            tool["inputSchema"]["type"] == "object",
            "{name} has no object inputSchema: {tool}"
        );
        let description = tool["description"].as_str().unwrap_or("");
        assert!(!description.is_empty(), "{name} has no description");
    }
    h.stop();
}

#[tokio::test]
async fn tools_list_requires_a_token() {
    let h = Harness::start().await;
    let response = h
        .post(
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
            None,
        )
        .await;
    assert_eq!(response.status(), 401);
    h.stop();
}

// ---------------------------------------------------------------------------
// tools/call
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_call_job_create_returns_a_real_job_id() {
    // End to end: HTTP → rmcp → dispatch → runtime → store → back
    // as JSON. The id must be a real one the store will accept,
    // which is what a client does next with `job.get`.
    let h = Harness::start().await;
    let response = h
        .post(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "job.create",
                    "arguments": {
                        "orchestrator": {"kind": "ironclaw"},
                        "package": {
                            "distribution": {"family": "debian", "release": "sid"},
                            "source_name": "fixture-pkg",
                            "binary_names": [],
                        },
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("json");

    assert_eq!(body["error"], Value::Null, "no error expected: {body}");
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let job_id = body["result"]["structuredContent"]["job_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no structured job_id: {body}"))
        .to_string();
    assert!(!job_id.is_empty());
    assert!(text.contains(&job_id), "text and structured must agree");

    // And the id round-trips: a follow-up call must find the job.
    let response = h
        .post(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {"name": "job.get", "arguments": {"job_id": job_id}},
            }),
            Some(TOKEN),
        )
        .await;
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["error"], Value::Null, "job.get failed: {body}");
    assert_eq!(
        body["result"]["structuredContent"]["projection"]["job"]["id"],
        job_id.as_str(),
        "job.get body: {body}"
    );
    assert_eq!(
        body["result"]["structuredContent"]["projection"]["state"], "event_detected",
        "a fresh job starts at event_detected: {body}"
    );
    h.stop();
}

#[tokio::test]
async fn a_failing_tool_returns_a_readable_error_not_a_protocol_error() {
    // MCP distinguishes the two. `Ok(CallToolResult::error)` is
    // rendered in the client's conversation; `Err(McpError)` becomes
    // an opaque "internal error". An agent that cannot read *why*
    // `operation.get` failed cannot recover, and a wrong-but-readable
    // message is strictly better than an unreadable one.
    let h = Harness::start().await;
    let response = h
        .post(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "operation.get",
                    "arguments": {
                        "operation_id": ironmaint_core::OperationId::new(),
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "a tool error is still HTTP 200");
    let body: Value = response.json().await.expect("json");

    assert_eq!(
        body["error"],
        Value::Null,
        "must not be a JSON-RPC protocol error: {body}"
    );
    assert_eq!(
        body["result"]["isError"], true,
        "must be flagged as a tool error: {body}"
    );
    let message = body["result"]["structuredContent"]["error"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(
        message.contains("unknown operation"),
        "the agent must be able to read what went wrong: {body}"
    );
    h.stop();
}

#[tokio::test]
async fn an_unknown_tool_is_reported_as_a_tool_error() {
    let h = Harness::start().await;
    let body: Value = h
        .post(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {"name": "job.teleport", "arguments": {}},
            }),
            Some(TOKEN),
        )
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert!(
        body["result"]["structuredContent"]["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown tool"),
        "{body}"
    );
    h.stop();
}

#[tokio::test]
async fn tools_call_requires_a_token() {
    let h = Harness::start().await;
    let response = h
        .post(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": {"name": "job.create", "arguments": {}},
            }),
            None,
        )
        .await;
    assert_eq!(
        response.status(),
        401,
        "no tool call may run unauthenticated"
    );
    h.stop();
}

#[tokio::test]
async fn the_server_issues_no_session_id() {
    // Stateless by construction. A session id in the response would
    // mean state accumulating per client, which for a local daemon
    // with one agent is pure cleanup burden — and `NeverSessionManager`
    // means a regression here fails loudly rather than silently
    // creating one.
    let h = Harness::start().await;
    let response = h.initialize(Some(TOKEN)).await;
    assert_eq!(response.status(), 200);
    assert!(
        response.headers().get("mcp-session-id").is_none(),
        "no session id may be issued"
    );
    h.stop();
}
