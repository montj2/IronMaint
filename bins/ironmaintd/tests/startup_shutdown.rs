//! Daemon lifecycle and end-to-end MCP tests (PHASE-0B.md §66-§68).
//!
//! These drive the real binary as a child process over a real
//! socket. The previous version of this file spawned the daemon,
//! slept 300 ms, killed it, and accepted any exit status — a
//! test that passes when the daemon is broken. What §102 items 23
//! and 26 actually claim is that an agent can connect, authenticate,
//! and call tools against this process, so that is what is
//! asserted here.
//!
//! `--bind 127.0.0.1:0` is used throughout so tests never collide
//! on a port; the daemon prints the address it bound on stdout,
//! which is the only way a caller could learn it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

const TOKEN: &str = "ironmaint-daemon-test-token-0123456789";
const LISTENING_PREFIX: &str = "ironmaintd listening on ";

/// rmcp requires both media types in `Accept` on a POST; a request
/// offering one is rejected 406 before dispatch.
const ACCEPT: &str = "application/json, text/event-stream";

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn fixture_bin() -> PathBuf {
    // Anchored on the daemon's own path, not this test binary's:
    // `current_exe()` in a test resolves to `target/debug/deps/`.
    let path = PathBuf::from(env!("CARGO_BIN_EXE_ironmaintd"))
        .parent()
        .expect("target dir")
        .join("ironmaint-fixture");
    assert!(
        path.is_file(),
        "{} is missing — the daemon's check.run has nothing to execute. \
         `cargo test --workspace` builds it; `cargo test -p ironmaintd` alone may not.",
        path.display()
    );
    path
}

/// The full argument list for a daemon rooted at `tmp`.
///
/// Every test that needs a *second* daemon must go through this,
/// so that "the same state directory" means what it says. An
/// earlier version of this file built the second daemon's paths
/// by hand, pointed it at its own fresh tempdir, and so was
/// really asserting that two daemons on two different
/// directories do not collide.
fn command_for(tmp: &tempfile::TempDir) -> tokio::process::Command {
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).expect("state dir");
    let token_file = tmp.path().join("token");
    std::fs::write(&token_file, TOKEN).expect("token");
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironmaintd"));
    command
        .arg("--state-dir")
        .arg(&state)
        .arg("--token-file")
        .arg(&token_file)
        .arg("--migrations-dir")
        .arg(migrations_dir())
        .arg("--workspace-root")
        .arg(tmp.path().join("workspaces"))
        .arg("--artifacts-root")
        .arg(tmp.path().join("artifacts"))
        .arg("--fixture-bin")
        .arg(fixture_bin())
        .arg("--bind")
        .arg("127.0.0.1:0");
    command
}

/// A running daemon, killed on drop.
struct Daemon {
    child: tokio::process::Child,
    url: String,
    /// Shared so a test can restart the daemon against the same
    /// state directory without moving a tempdir out of a struct
    /// that is about to be dropped.
    tmp: Arc<tempfile::TempDir>,
}

impl Daemon {
    /// Start the daemon on an ephemeral port and wait for it to
    /// report the address it bound.
    async fn start() -> Self {
        Self::start_on(Arc::new(tempfile::tempdir().expect("tempdir"))).await
    }

    async fn start_on(tmp: Arc<tempfile::TempDir>) -> Self {
        let mut command = command_for(&tmp);
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ironmaintd");

        // Read stdout until the listening line. Anything else on
        // stdout would break this, which is why logs go to stderr.
        let stdout = child.stdout.take().expect("piped stdout");
        let mut lines = BufReader::new(stdout).lines();
        let url = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(rest) = line.strip_prefix(LISTENING_PREFIX) {
                    return Some(rest.to_string());
                }
            }
            None
        })
        .await
        .unwrap_or(None)
        .unwrap_or_else(|| panic!("the daemon never reported a listening address"));

        Self { child, url, tmp }
    }

    async fn post(&self, body: Value, token: Option<&str>) -> reqwest::Response {
        let mut request = self
            .client()
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", ACCEPT)
            .header("Host", "127.0.0.1");
        if let Some(t) = token {
            request = request.header("Authorization", format!("Bearer {t}"));
        }
        request.json(&body).send().await.expect("send")
    }

    async fn call(&self, name: &str, arguments: Value) -> Value {
        let response = self
            .post(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {"name": name, "arguments": arguments},
                }),
                Some(TOKEN),
            )
            .await;
        assert_eq!(response.status(), 200, "tools/call {name} was not served");
        let body: Value = response.json().await.expect("json body");
        assert_eq!(
            body["error"],
            Value::Null,
            "protocol error calling {name}: {body}"
        );
        assert_eq!(
            body["result"]["isError"], false,
            "tool {name} failed: {}",
            body["result"]["structuredContent"]
        );
        body["result"]["structuredContent"].clone()
    }

    fn client(&self) -> reqwest::Client {
        reqwest::Client::new()
    }

    /// SIGTERM, then wait for exit.
    async fn terminate(&mut self) -> std::process::ExitStatus {
        // `Child::kill` sends SIGKILL, which is precisely the
        // path §68 exists to avoid: it cannot be drained. SIGTERM
        // is sent by name so the graceful path is what is tested.
        let pid = self.child.id().expect("pid");
        let status = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        assert!(
            status.is_ok_and(|s| s.success()),
            "could not send SIGTERM to {pid}"
        );
        tokio::time::timeout(Duration::from_secs(30), self.child.wait())
            .await
            .expect("the daemon must exit within 30s of SIGTERM")
            .expect("wait")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // A test that panicked mid-scenario must not leave a
        // daemon holding the state directory's lock, which would
        // make the *next* test fail for the wrong reason.
        let _ = self.child.start_kill();
    }
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_daemon_binds_a_socket_and_says_which_one() {
    // The line itself is the contract: the e2e script, the
    // operator doc, and these tests all learn the port from it.
    // With `--bind 127.0.0.1:0` there is no other way.
    let d = Daemon::start().await;
    assert!(d.url.starts_with("http://127.0.0.1:"), "{}", d.url);
    assert!(d.url.ends_with("/mcp"), "{}", d.url);
    let port: u16 = d
        .url
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches("/mcp").parse().ok())
        .expect("a port in the URL");
    assert_ne!(port, 0, "the daemon must report the port it actually got");
}

#[tokio::test]
async fn the_daemon_creates_its_directories_and_database() {
    let d = Daemon::start().await;
    let state = d.tmp.path().join("state");
    assert!(
        state.join("ironmaint.sqlite").is_file(),
        "no database was created"
    );
    // §10: exactly one daemon per state directory.
    assert!(
        state.join("ironmaint.lock").exists(),
        "no single-daemon lock"
    );
    // The roots the daemon was told to use, not a guessed
    // default layout: these tests pass them explicitly, precisely
    // so that nothing is written outside the tempdir.
    assert!(
        d.tmp.path().join("workspaces").is_dir(),
        "no workspace root"
    );
    assert!(d.tmp.path().join("artifacts").is_dir(), "no artifact root");
}

#[tokio::test]
async fn a_second_daemon_on_the_same_state_dir_refuses_to_start() {
    // §10. The failure must be immediate and loud: two daemons
    // sharing a state directory race on the same workspaces and
    // the same executor, and the loser must not run anyway.
    let d = Daemon::start().await;
    // Same tempdir, therefore the same `--state-dir`. §10's rule
    // is about one daemon per *state directory*, so the second
    // process has to be pointed at the first one's directory or
    // this asserts nothing.
    let output = command_for(&d.tmp)
        .output()
        .await
        .expect("run second daemon");
    assert!(!output.status.success(), "a second daemon must not start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lock") || stderr.contains("store"),
        "the refusal must say why: {stderr}"
    );
    // The first daemon is unaffected.
    let body = d.call("job.create", create_job_args()).await;
    assert!(
        body["job_id"].is_string(),
        "the first daemon stopped working: {body}"
    );
}

// ---------------------------------------------------------------------------
// Refusals at startup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_daemon_with_no_token_exits_non_zero_and_names_the_flag() {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironmaintd"))
        .arg("--state-dir")
        .arg(tempfile::tempdir().expect("tmp").path())
        .output()
        .await
        .expect("run");
    assert_eq!(output.status.code(), Some(1), "must refuse to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--token-file"), "{stderr}");
    assert!(
        stderr.contains("no unauthenticated mode"),
        "the reason must be stated: {stderr}"
    );
}

#[tokio::test]
async fn an_unknown_flag_exits_non_zero_and_suggests_help() {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironmaintd"))
        .arg("--serve-everything")
        .output()
        .await
        .expect("run");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--serve-everything"), "{stderr}");
    assert!(
        stderr.contains("--help"),
        "the fix must be suggested: {stderr}"
    );
}

#[tokio::test]
async fn help_prints_usage_on_stdout_and_exits_zero() {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironmaintd"))
        .arg("--help")
        .output()
        .await
        .expect("run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--token-file"), "{stdout}");
    assert!(
        stdout.contains(LISTENING_PREFIX),
        "usage must document the listening line: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// The socket (§102 items 23 and 26)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_authenticated_initialize_is_answered() {
    let d = Daemon::start().await;
    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "ironmaintd-test", "version": "1.0"},
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18", "{body}");
    assert!(
        body["result"]["capabilities"]["tools"].is_object(),
        "{body}"
    );
}

#[tokio::test]
async fn an_unauthenticated_call_is_401() {
    let d = Daemon::start().await;
    let response = d
        .post(
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
            None,
        )
        .await;
    assert_eq!(
        response.status(),
        401,
        "the daemon must not serve an anonymous caller"
    );
}

#[tokio::test]
async fn the_daemon_advertises_all_nine_tools() {
    // §94 enumerates nine, and the e2e script hardcodes the same
    // list. A missing tool here is the exact failure that made
    // `ironclaw-e2e.sh` unrunnable.
    let d = Daemon::start().await;
    let response = d
        .post(
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
            Some(TOKEN),
        )
        .await;
    let body: Value = response.json().await.expect("json");
    let mut names: Vec<String> = body["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "candidate.capture",
            "check.run",
            "job.create",
            "job.get",
            "job.next_actions",
            "job.reconcile",
            "operation.get",
            "workspace.apply_patch",
            "workspace.stat",
        ]
    );
}

// ---------------------------------------------------------------------------
// End to end, through the process
// ---------------------------------------------------------------------------

fn create_job_args() -> Value {
    json!({
        "orchestrator": {"kind": "ironclaw"},
        "package": {
            "distribution": {"family": "debian", "release": "sid"},
            "source_name": "ironmaint-daemon-fixture",
            "binary_names": [],
        },
    })
}

#[tokio::test]
async fn a_job_created_over_http_is_persisted_and_readable_back() {
    // The load-bearing end-to-end claim: an agent talks to this
    // process, and what it creates is real state on disk, not a
    // response object that vanishes when the connection closes.
    let d = Daemon::start().await;
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no job_id in {created}"))
        .to_string();

    // Readable back over a *second* HTTP request, from a second
    // connection, in a different keep-alive session. What proves
    // it was actually persisted rather than held in the request is
    // `a_second_daemon_reuses_the_persisted_database` below,
    // which stops this process and reads the same job back with a
    // new one.
    let fetched = d.call("job.get", json!({"job_id": job_id})).await;
    assert_eq!(
        fetched["projection"]["job"]["id"],
        job_id.as_str(),
        "job.get must return the job that was created: {fetched}"
    );
    assert_eq!(fetched["projection"]["state"], "event_detected");

    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .unwrap_or_else(|| panic!("no allowed array in {next}"));
    assert!(
        !allowed.is_empty(),
        "a fresh job must offer allowed actions: {next}"
    );
    assert_eq!(
        allowed[0], "capture_candidate",
        "the first move a fresh job may take is to capture a candidate: {next}"
    );
}

#[tokio::test]
async fn a_failing_tool_call_is_reported_readably_rather_than_as_a_crash() {
    // The daemon's job is to keep serving after an agent makes a
    // mistake. A missing job id is the most likely one.
    let d = Daemon::start().await;
    let body: Value = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "job.get",
                    "arguments": {"job_id": "00000000-0000-7000-8000-000000000000"},
                },
            }),
            Some(TOKEN),
        )
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(
        body["error"],
        Value::Null,
        "must not be a protocol error: {body}"
    );
    assert_eq!(body["result"]["isError"], true, "{body}");
    let message = body["result"]["structuredContent"]["error"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(
        !message.is_empty(),
        "the agent must be told what went wrong: {body}"
    );

    // Still serving.
    let created = d.call("job.create", create_job_args()).await;
    assert!(created["job_id"].is_string(), "{created}");
}

#[tokio::test]
async fn a_second_daemon_reuses_the_persisted_database() {
    // Restart behaviour, and the reason the daemon opens a file
    // rather than an in-memory store: state must survive the
    // process.
    let tmp = Arc::new(tempfile::tempdir().expect("tmp"));
    let job_id = {
        let mut d = Daemon::start_on(Arc::clone(&tmp)).await;
        let created = d.call("job.create", create_job_args()).await;
        let id = created["job_id"].as_str().expect("job_id").to_string();
        // Shut the first one down cleanly: §10's lock is held for
        // the process lifetime, so the replacement must wait for
        // a graceful exit rather than a kill.
        let status = d.terminate().await;
        assert!(status.success(), "SIGTERM must exit 0, got {status:?}");
        id
    };

    let d = Daemon::start_on(tmp).await;
    // Readable back over a *second* HTTP request, from a second
    // connection, in a different keep-alive session. What proves
    // it was actually persisted rather than held in the request is
    // `a_second_daemon_reuses_the_persisted_database` below,
    // which stops this process and reads the same job back with a
    // new one.
    let fetched = d.call("job.get", json!({"job_id": job_id})).await;
    assert_eq!(
        fetched["projection"]["job"]["id"],
        job_id.as_str(),
        "the job must survive a restart: {fetched}"
    );
}

// ---------------------------------------------------------------------------
// Shutdown (§68)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sigterm_drains_and_exits_zero() {
    // Not "exits somehow". §68 requires a graceful drain, and
    // SIGKILL — which is what `Child::kill` sends — is the path
    // that requirement exists to distinguish from this one.
    let mut d = Daemon::start().await;
    let created = d.call("job.create", create_job_args()).await;
    assert!(created["job_id"].is_string());
    let status = d.terminate().await;
    assert_eq!(
        status.code(),
        Some(0),
        "a graceful drain must exit 0: {status:?}"
    );
}
