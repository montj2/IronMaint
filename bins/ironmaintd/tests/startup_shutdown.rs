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

/// The fixture binary the daemon's `check.run` will execute.
///
/// Resolved by the crate that owns the binary, which also checks the
/// candidate is loadable on this host. The previous version anchored
/// on the daemon's own path and joined a sibling, which held for the
/// *daemon* — `CARGO_BIN_EXE_ironmaintd` is set because the daemon is
/// in this package, and it is freshly built for this host. The sibling
/// had neither property: `ironmaintd` does not depend on the fixture
/// crate, so cargo does not rebuild it, and a `target/` shared with
/// another platform's build satisfies the `is_file()` check with a
/// binary that cannot be executed here. The daemon then reports a tool
/// that exits 127, which is not a shape any test in this file could
/// have attributed to its cause.
fn fixture_bin() -> PathBuf {
    ironmaint_fixture::fixture_binary_path()
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
        .arg("--debian-tool-bin")
        .arg(ironmaint_debian_tool::binary_path())
        .arg("--bind")
        .arg("127.0.0.1:0");
    // Without this, dropping the `Child` neither kills nor reaps:
    // the default is `false`, and the only thing that made a
    // dropped daemon die was the explicit `start_kill` in
    // `Drop for Daemon`, which signals but cannot wait. Setting it
    // makes the panic path — an assertion fires mid-test and the
    // `Daemon` goes out of scope — kill *and* return the process to
    // the reaper, so a failing test does not leave a live daemon
    // holding a state directory for the rest of the run.
    command.kill_on_drop(true);
    command
}

/// A running daemon, reaped on drop and on an explicit [`Daemon::shutdown`].
struct Daemon {
    child: tokio::process::Child,
    url: String,
    /// Shared so a test can restart the daemon against the same
    /// state directory without moving a tempdir out of a struct
    /// that is about to be dropped.
    tmp: Arc<tempfile::TempDir>,
}

impl Daemon {
    /// Path of the per-job workspace root the daemon was
    /// started with. Tests use it to locate the
    /// `<handle>/` directory a `candidate.capture` call
    /// provisions and to stage fixtures into it.
    fn workspaces_root(&self) -> std::path::PathBuf {
        self.tmp.path().join("workspaces")
    }

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

    /// SIGTERM, then wait for exit, and hand back the status.
    ///
    /// This is the strict form, for the two tests that are *about*
    /// §68's graceful drain and therefore assert on the result. Every
    /// other test wants [`Daemon::shutdown`], which does not care how
    /// the daemon stopped as long as it did.
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

    /// Stop the daemon and **reap** it, escalating if it overstays.
    ///
    /// The distinction from [`Daemon::terminate`] is that this one
    /// does not care how the daemon ended. Every test that merely
    /// *used* the daemon should call it, because leaving the reaping
    /// to `Drop` leaves it to tokio's SIGCHLD orphan handler — and
    /// that handler is exactly what stops working when the
    /// environment is already unhealthy, which is the state in which
    /// a leaked child does the most damage. Eleven of the fifteen
    /// tests in this file relied on that path alone.
    async fn shutdown(&mut self) {
        // Already gone is the outcome we wanted, not a failure: a
        // test that made the daemon exit on its own must not report
        // an error for having done so.
        if self.child.try_wait().expect("try_wait").is_some() {
            return;
        }
        if let Some(pid) = self.child.id() {
            // Best effort. A `kill` that fails because the process
            // exited between the `try_wait` and the signal is
            // harmless, and the timeout below turns the genuinely
            // stuck case into a kill rather than a hang.
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status();
        }
        // 10s rather than the 30s above: this path only runs when a
        // test is finishing, so the only thing being bought is a
        // clean exit between tests, not a correctness assertion.
        if tokio::time::timeout(Duration::from_secs(10), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.start_kill();
            // The reap that `Drop` cannot perform, and that the
            // orphan handler would eventually perform on its own
            // schedule. Doing it here means the next test starts from
            // a process table with nothing left in it.
            let _ = self.child.wait().await;
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // The panic path, and only that. `start_kill` signals and
        // returns; `kill_on_drop(true)` on the `Command` in
        // `command_for` is what turns the subsequent field drop into
        // a reap. A test that reaches its end calls `shutdown` and
        // never lands here having run to completion.
        //
        // A test that panicked mid-scenario must not leave a daemon
        // holding the state directory's lock, which would make the
        // *next* test fail for the wrong reason.
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
    let mut d = Daemon::start().await;
    assert!(d.url.starts_with("http://127.0.0.1:"), "{}", d.url);
    assert!(d.url.ends_with("/mcp"), "{}", d.url);
    let port: u16 = d
        .url
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches("/mcp").parse().ok())
        .expect("a port in the URL");
    assert_ne!(port, 0, "the daemon must report the port it actually got");
    d.shutdown().await;
}

#[tokio::test]
async fn the_daemon_creates_its_directories_and_database() {
    let mut d = Daemon::start().await;
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
    d.shutdown().await;
}

#[tokio::test]
async fn a_second_daemon_on_the_same_state_dir_refuses_to_start() {
    // §10. The failure must be immediate and loud: two daemons
    // sharing a state directory race on the same workspaces and
    // the same executor, and the loser must not run anyway.
    let mut d = Daemon::start().await;
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
    d.shutdown().await;
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
    let mut d = Daemon::start().await;
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
    d.shutdown().await;
}

#[tokio::test]
async fn an_unauthenticated_call_is_401() {
    let mut d = Daemon::start().await;
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
    d.shutdown().await;
}

#[tokio::test]
async fn the_daemon_advertises_every_tool_the_server_registers() {
    // §94 enumerates tool *capabilities* — "reconcile", "next-actions",
    // "candidate capture", and so on — not a count of nine, so a tenth
    // or eleventh tool does not contradict it. The tenth, `job.resume`
    // (0B.10 C2), exists because an agent that lands in
    // `HumanReviewRequired` otherwise has no way out; the eleventh,
    // `release.candidate.create` (0B.10 C5), exists because §101
    // steps 26-27 are not expressible without it. The twelfth,
    // thirteenth, and fourteenth — `evidence.list`, `evidence.get`,
    // and `evidence.artifact.read` (PHASE-1.md §31 / PR 1A.4) —
    // exist because 1A.3 writes `Evidence` rows with bound report
    // artifacts, and an MCP client that ran `check.run` needs a way
    // to read them back; without these the §31 exit-checkpoint
    // ("have an MCP client read the report") is unmet.
    //
    // The e2e script hardcodes the same list. A missing tool here is
    // the exact failure that made `ironclaw-e2e.sh` unrunnable.
    let mut d = Daemon::start().await;
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
            "evidence.artifact.read",
            "evidence.get",
            "evidence.list",
            "job.create",
            "job.get",
            "job.next_actions",
            "job.reconcile",
            "job.resume",
            "operation.get",
            "release.candidate.create",
            "workspace.apply_patch",
            "workspace.stat",
        ]
    );
    d.shutdown().await;
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
    let mut d = Daemon::start().await;
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
    d.shutdown().await;
}

#[tokio::test]
async fn a_failing_tool_call_is_reported_readably_rather_than_as_a_crash() {
    // The daemon's job is to keep serving after an agent makes a
    // mistake. A missing job id is the most likely one.
    let mut d = Daemon::start().await;
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
    d.shutdown().await;
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

    let mut d = Daemon::start_on(tmp).await;
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
    d.shutdown().await;
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

#[tokio::test]
async fn a_shutdown_reaps_the_daemon_rather_than_merely_signalling_it() {
    // The property every other test in this file now depends on, and
    // the one the old `Drop` did not have: when `shutdown` *returns*,
    // the process is gone from the kernel's table, not merely told to
    // go away.
    //
    // `Drop` could never assert this. It can only call `start_kill`,
    // which delivers SIGKILL and returns before the target has
    // certainly processed it, and it cannot call `wait` at all — so
    // the reap fell to tokio's SIGCHLD orphan handler on whatever
    // schedule the handler got to it. That is fine while everything
    // is healthy and useless precisely when it is not.
    let mut d = Daemon::start().await;
    let pid = d.child.id().expect("the daemon has a pid");
    assert!(process_exists(pid), "the daemon should be running");

    d.shutdown().await;

    // `kill -0` is the kernel's own answer to "does this pid exist",
    // and it distinguishes a live process from a zombie: a zombie is
    // still in the table, which is the state `start_kill` alone would
    // leave behind. A reaped child is gone entirely.
    assert!(
        !process_exists(pid),
        "{pid} is still in the process table after shutdown returned; \
         shutdown must wait for the exit, not just request it"
    );
}

/// Whether `pid` is still in the kernel's process table — true for a
/// running process *and* for a zombie, false only once it has been
/// waited for.
fn process_exists(pid: u32) -> bool {
    // Signal 0 performs the permission and existence checks without
    // delivering anything. `std::process::Command` rather than a raw
    // `kill(2)` because the workspace forbids `unsafe`. stderr is
    // discarded because the answer for a gone pid is non-zero, and
    // `kill` narrates that on stderr, which is not a test failure and
    // does not belong in the output.
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------------------
// The adapter registry (§67), over the wire
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_captured_candidate_is_activated_with_its_adapter_derived_gates() {
    // The D-03 test, at the level the agent actually meets it.
    // Before 0B.10 C1, `candidate.capture` persisted a row and
    // stopped: nothing set the active candidate, so `reconcile`
    // reported `NoOp` forever and an agent could capture candidates
    // until it ran out of context without the job moving.
    //
    // Now capture activates the candidate and asks the registered
    // `debian-stub` adapter what the candidate requires. The gates
    // it derives name `debian.*` tools, which are not executable in
    // 0B — but `next_actions` naming them, rather than silence, is
    // the whole difference this test pins.
    let mut d = Daemon::start().await;
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no job_id in {created}"))
        .to_string();

    let captured = d
        .call(
            "candidate.capture",
            json!({
                "job_id": job_id,
                "package": {
                    "distribution": {"family": "debian", "release": "sid"},
                    "source_name": "ironmaint-daemon-fixture",
                    "binary_names": [],
                },
                "repository_url": "https://example.invalid/ironmaint-daemon-fixture.git",
            }),
        )
        .await;
    let fingerprint = captured["fingerprint"]
        .as_str()
        .unwrap_or_else(|| panic!("no fingerprint in {captured}"))
        .to_string();
    assert!(!fingerprint.is_empty(), "{captured}");

    // The capture narrates what it did, so an agent can tell an
    // inert capture from a working one.
    let notes: Vec<&str> = captured["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("no notes array in {captured}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        notes.iter().any(|n| n.contains("activated fingerprint")),
        "capture must report the activation: {notes:?}"
    );
    assert!(
        notes.iter().any(|n| n.contains("materialized 5 check(s)")),
        "the production Debian adapter (1B.3 + 1C.1 + 1C.2 + 1C.3 + 1D.2 + 1E.2) plans five checks \
         (`debian.inspect.source_preparation`, `debian.inspect.source_analysis`, \
         `debian.inspect.maintenance_context`, `debian.policy.delta_review`, and \
         `debian.issue.bts_snapshot`); all must reach the store: {notes:?}"
    );
    // 1D.2: the production Debian adapter now
    // advertises `PolicyDerivation` (1D.1's manifest
    // is the policy-knowledge baseline) and
    // `DebianPolicyDerivation::derive_obligation_plan`
    // produces exactly one mandatory
    // `Standards-Version` obligation. The runtime
    // logs the count as "derived 1 obligation(s)
    // for fingerprint <fp>"; the test pins the
    // count == 1 (not zero, not two) so the §32
    // "policy obligation derivation is
    // deliberately limited" property holds.
    assert!(
        notes
            .iter()
            .any(|n| n.contains("derived 1 obligation(s) for fingerprint")),
        "1D.2's `DebianPolicyDerivation` must derive exactly one \
         mandatory `Standards-Version` obligation; a count of 0 means \
         the policy_cap module is not wired, a count of 2+ means the \
         adapter is over-deriving: {notes:?}"
    );

    // 1. The job now knows which candidate it is evaluating.
    let fetched = d.call("job.get", json!({"job_id": job_id})).await;
    assert!(
        fetched["projection"]["active_candidate"].is_string(),
        "capture must activate the candidate, or nothing downstream can \
         be candidate-scoped: {fetched}"
    );

    // 2. Every derived gate is actionable, by check id — which is
    //    the only form `check.run` accepts. The agent is still at
    //    `EventDetected`, and it must be able to run the build
    //    check there: §101 does exactly that, four candidates deep,
    //    before the job advances a single state.
    //
    //    The `blockers` array is empty at this state, and that is
    //    correct rather than a gap: the state machine has no
    //    *pending gate* until the job reaches `SourceIntegrity`, so
    //    there is nothing for `GatePending` to say yet. Runnability
    //    is a fact about the store; pendingness is a fact about the
    //    state machine, and only the latter waits for `reconcile`.
    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .unwrap_or_else(|| panic!("no allowed array in {next}"));
    let run_checks: Vec<&str> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .collect();
    assert_eq!(
        run_checks.len(),
        5,
        "the production Debian adapter (1B.3 + 1C.1 + 1C.2 + 1C.3 + 1D.2 + 1E.2) plans five checks \
         (`debian.inspect.source_preparation`, `debian.inspect.source_analysis`, \
         `debian.inspect.maintenance_context`, `debian.policy.delta_review`, and \
         `debian.issue.bts_snapshot`); all must be offered as `run_check` actions: {next}"
    );
    assert!(
        run_checks.iter().all(|id| !id.is_empty()),
        "each offered check must carry the id check.run needs: {next}"
    );

    // 3. Running one of them reports the gate as blocked. From 1C.1
    //    onward, the tool is *registered* (it is the production
    //    `debian.inspect.source_preparation` entry); on a build
    //    that does not yet include the binary, the executor's
    //    pre-flight `check_launchable` rejects the path and the
    //    evidence row is `infrastructure_error` with the gate
    //    blocked. The previous 1B.3 RED state — "the tool is
    //    completely unknown, so `check.run` is a tool error" —
    //    became obsolete the moment 1C.1 RED registered the tool;
    //    the RED→GREEN transition is then between
    //    `infrastructure_error` (no binary) and
    //    `pass`/`fail` (the real binary, on a real fixture).
    let body: Value = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {"job_id": job_id, "check_id": run_checks[0]},
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
        "an unrunnable tool is a tool-level outcome, not a protocol error: {body}"
    );
    let structured = &body["result"]["structuredContent"];
    // 1C.1 GREEN: the binary is real. The empty
    // workspace (no `debian/` directory) drives the
    // tool's verdict to `fail` and the normalizer to
    // `EvidenceStatus::Fail` — the tool ran, the
    // *content* is the problem, not the toolchain.
    // The two new tests below cover both
    // `evidence_status == "pass"` (good fixture) and
    // `evidence_status == "fail"` (mismatched fixture).
    assert_eq!(
        structured["evidence_status"], "fail",
        "an empty workspace has no `debian/` directory; the source_preparation \
         tool's verdict is `fail` (the §17 fail path, distinct from \
         `infrastructure_error`). The 1B.3/1C.1 RED state \
         (`infrastructure_error` because the binary was missing) becomes \
         obsolete once 1C.1 GREEN installs the real binary: {structured}"
    );
    assert_eq!(
        structured["gate_status"], "fail",
        "a fail propagates to the gate; the agent sees a concrete fail, not \
         a silent stall: {structured}"
    );
    assert!(
        structured["check_id"].is_string(),
        "the response must carry the check_id the runtime used: {structured}"
    );
    assert!(
        structured["evidence_id"].is_string(),
        "the response must carry the evidence_id of the new evidence row, so \
         the agent can read the report (PHASE-1.md §31): {structured}"
    );
    d.shutdown().await;
}

#[tokio::test]
async fn a_family_the_daemon_has_no_adapter_for_still_captures() {
    // The registry is keyed by family, and a miss must be a
    // diagnostic, not a failure: refusing the capture would make a
    // candidate for an unknown distribution unrepresentable.
    let mut d = Daemon::start().await;
    let created = d
        .call(
            "job.create",
            json!({
                "orchestrator": {"kind": "ironclaw"},
                "package": {
                    "distribution": {"family": "arch", "release": "rolling"},
                    "source_name": "ironmaint-daemon-fixture",
                    "binary_names": [],
                },
            }),
        )
        .await;
    let job_id = created["job_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no job_id in {created}"))
        .to_string();

    let captured = d
        .call(
            "candidate.capture",
            json!({
                "job_id": job_id,
                "package": {
                    "distribution": {"family": "arch", "release": "rolling"},
                    "source_name": "ironmaint-daemon-fixture",
                    "binary_names": [],
                },
                "repository_url": "https://example.invalid/ironmaint-daemon-fixture.git",
            }),
        )
        .await;
    assert!(
        captured["fingerprint"].is_string(),
        "an unregistered family must still capture: {captured}"
    );

    // Activation does not depend on the adapter, so the job is
    // still evaluable — it simply has no derived gates.
    let fetched = d.call("job.get", json!({"job_id": job_id})).await;
    assert!(
        fetched["projection"]["active_candidate"].is_string(),
        "activation is a fact about the job, not about the adapter: {fetched}"
    );
    d.shutdown().await;
}

// ---------------------------------------------------------------------
// 1C.1 GREEN: `debian.inspect.source_preparation` is the first
// non-synthetic Phase 1 check. The two tests below drive the
// hermetic fixtures through a real daemon and assert the tool's
// verdict on the workspace (Pass for a well-formed Debian source
// tree, Fail for a tree whose changelog's source name disagrees
// with the candidate). The earlier 1B.3 test above asserts the
// fail-when-empty-workspace path.
// ---------------------------------------------------------------------

/// Recursively copy a directory tree, used to stage the
/// hermetic fixtures into the workspace directory the
/// daemon allocated.
fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) {
    if src.is_dir() {
        std::fs::create_dir_all(dst).expect("mkdir copy dst");
        for entry in std::fs::read_dir(src).expect("readdir src") {
            let entry = entry.expect("readdir entry");
            let from = entry.path();
            let to = dst.join(entry.file_name());
            copy_recursive(&from, &to);
        }
    } else {
        std::fs::copy(src, dst).expect("copy file");
    }
}

/// Build a candidate-capture payload that uses the
/// `debian` family (so the production `adapters/debian/`
/// adapter — registered in 1B.3 — plans the
/// `debian.inspect.source_preparation` check) and the
/// given `source_name`. The `expected_version` field is
/// left unset so the runtime resolves the candidate's
/// `source_version` from the workspace's
/// `debian/changelog` (PHASE-1.1 PR 1.1.A). The hermetic
/// fixture's `debian/changelog` is written to declare a
/// real Debian-style version so the cross-check between
/// the changelog and the candidate's resolved version is
/// a real assertion of identity, not a tautology against
/// a fixed placeholder.
fn candidate_capture_args(job_id: &str, source_name: &str) -> Value {
    candidate_capture_args_with_version(job_id, source_name, None)
}

/// Like `candidate_capture_args` but lets the caller pin
/// the version the runtime will use as the candidate's
/// `source_version`. When `expected_version` is `Some`,
/// the runtime records that value on the candidate and
/// the `debian.inspect.source_preparation` check emits
/// an `E_VERSION_MISMATCH` diagnostic if the changelog's
/// first entry disagrees. When `None`, the runtime reads
/// the version from the workspace's `debian/changelog`.
fn candidate_capture_args_with_version(
    job_id: &str,
    source_name: &str,
    expected_version: Option<&str>,
) -> Value {
    let mut args = json!({
        "job_id": job_id,
        "package": {
            "distribution": {"family": "debian", "release": "sid"},
            "source_name": source_name,
            "binary_names": [],
        },
        "repository_url": "https://example.invalid/ironmaint-debian-tool-fixture.git",
    });
    if let Some(v) = expected_version {
        args["expected_version"] = json!(v);
    }
    args
}

/// Provision the job's workspace, stage the named
/// hermetic fixture into it, and capture a candidate
/// against the staged tree. Returns the active
/// candidate's `check_id` (the `source_preparation`
/// check, which is the first thing `next_actions`
/// advertises).
///
/// The two-capture pattern is required because the
/// daemon's `ensure_workspace` is what creates the
/// `<workspace_root>/<handle>/` directory the fixture
/// has to be staged into. The first `candidate.capture`
/// provisions the empty workspace; the fixture is then
/// copied in; the second `candidate.capture` reads the
/// staged tree — including the changelog that drives
/// the version resolution (PHASE-1.1 PR 1.1.A). Without
/// the second capture, the candidate's
/// `source_version` would be derived from the empty
/// pre-stage workspace, not the real Debian source
/// tree the check ultimately inspects.
async fn capture_against_staged_fixture(
    d: &mut Daemon,
    job_id: &str,
    source_name: &str,
    fixture_relpath: &str,
) -> String {
    // 1. Provision the workspace via an initial
    //    capture. The candidate is created but its
    //    `source_version` is the placeholder fallback
    //    (empty workspace has no `debian/changelog`).
    let _ = d
        .call(
            "candidate.capture",
            candidate_capture_args(job_id, source_name),
        )
        .await;

    // 2. Locate the (only) workspace handle and stage
    //    the fixture into it.
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(
        handles.len(),
        1,
        "expected exactly one workspace handle; got {handles:?}"
    );
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian")
        .join(fixture_relpath);
    copy_recursive(&fixture_root, &workspace_path);

    // 3. Capture again — this time the workspace has
    //    the real Debian source tree, so
    //    `capture_candidate` reads the changelog and
    //    resolves the version from it. The runtime
    //    activates the new candidate, which is the one
    //    `next_actions` will report.
    let _ = d
        .call(
            "candidate.capture",
            candidate_capture_args(job_id, source_name),
        )
        .await;

    // 4. Pull the check_id from `next_actions`.
    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    next["actions"]["allowed"]
        .as_array()
        .and_then(|a| {
            a.iter().find_map(|a| {
                a.get("run_check")?
                    .get("check_id")?
                    .as_str()
                    .map(String::from)
            })
        })
        .expect("job.next_actions should list a run_check action for the active candidate")
}

/// The `debian.inspect.source_preparation` check passes
/// when the candidate's workspace contains a well-formed
/// Debian 3.0 (quilt) source tree whose `Source:`,
/// `Version:`, and `debian/source/format` all match.
#[tokio::test]
async fn debian_inspect_source_preparation_passes_against_the_hermetic_fixture() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let check_id = capture_against_staged_fixture(&mut d, &job_id, "example", "example-1.0").await;

    // Run the check. The MCP `tools/call` response wraps
    // the `RunCheckOutput` in `body["result"]` (not
    // `body["result"]["structuredContent"]`); the
    // higher-level `Daemon::call` helper unwraps
    // `structuredContent` for tools that emit it, but
    // `check.run` emits the run output at the result
    // root. Use `Daemon::post` and inspect the JSON-RPC
    // response directly.
    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": check_id.clone(),
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "check.run was not served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["error"],
        Value::Null,
        "check.run returned a protocol error: {body}"
    );
    let result = &body["result"]["structuredContent"];
    assert_eq!(
        result["evidence_status"], "pass",
        "the good fixture's §17 checks all hold; the source_preparation tool's \
         verdict is `pass` and the normalizer maps it to EvidenceStatus::Pass: {result}"
    );
    assert_eq!(
        result["gate_status"], "pass",
        "a pass lets the gate open: {result}"
    );

    d.shutdown().await;
}

/// The candidate's `source_version` is bound to the
/// `debian/changelog` first entry, not the historical
/// `0+ironmaint` placeholder (PHASE-1.1 PR 1.1.A —
/// P1 finding #1 from `doc/PHASE-1-FEEDBACK.md`).
///
/// The test stages the good hermetic fixture (whose
/// `debian/changelog` declares version `1.0.0-1`),
/// captures a candidate with no `expected_version`,
/// runs the `source_preparation` check, and asserts
/// the report's `changelog_version_matches_candidate`
/// is `true` — which only holds if the runtime read the
/// version from the changelog. With the historical
/// hard-coded `0+ironmaint` body the cross-check fails
/// and this test fails (RED).
#[tokio::test]
async fn candidate_capture_uses_real_changelog_version() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let check_id = capture_against_staged_fixture(&mut d, &job_id, "example", "example-1.0").await;

    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": check_id.clone(),
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "check.run was not served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["error"],
        Value::Null,
        "check.run returned a protocol error: {body}"
    );
    let result = &body["result"]["structuredContent"];
    let evidence_id = result["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();

    // Read the bound report so we can assert the
    // version cross-check held. The report's
    // `changelog_version` is what the parser read from
    // the fixture; `changelog_version_matches_candidate`
    // is the cross-check between that and the candidate's
    // `package.version` (what `capture_candidate` set).
    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": {"evidence_id": evidence_id.clone()},
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(get_response.status(), 200, "evidence.get was not served");
    let get_body: Value = get_response.json().await.expect("json body");
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array on the Evidence row");
    assert!(!artifacts.is_empty(), "executor binds one report artifact");
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id string")
        .to_string();

    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(artifact_response.status(), 200, "read was not served");
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    let bytes_base64 = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine as _;
    let artifact_bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64)
        .expect("base64-decode artifact bytes");
    let report: Value = serde_json::from_slice(&artifact_bytes).expect("report JSON");

    assert_eq!(
        report["findings"]["changelog_version"].as_str(),
        Some("1.0.0-1"),
        "the fixture's debian/changelog declares 1.0.0-1; the parser must surface that"
    );
    assert_eq!(
        report["findings"]["changelog_version_matches_candidate"], true,
        "candidate's package.version must match the changelog's first entry; \
         with the historical hard-coded `0+ironmaint` this is false (RED): {report}"
    );
    assert_eq!(
        result["evidence_status"], "pass",
        "the version cross-check holds only when capture_candidate read the \
         changelog, so the source_preparation tool's verdict is `pass`: {result}"
    );

    d.shutdown().await;
}

/// A caller-supplied `expected_version` wins over the
/// `debian/changelog`'s first entry. The runtime records
/// the supplied value as the candidate's `source_version`
/// and the `debian.inspect.source_preparation` check
/// records an `E_VERSION_MISMATCH` diagnostic when the
/// supplied value disagrees with the changelog
/// (PHASE-1.1 PR 1.1.A). The capture itself succeeds;
/// the verdict is `fail` only because the cross-check
/// on the §17 identity assertion failed — the
/// `evidence_status` surfaces the cross-check's
/// `Verdict::Fail` to the runtime.
#[tokio::test]
async fn candidate_capture_uses_supplied_expected_version() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();

    // First capture: provision the workspace.
    let _ = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // Stage the good fixture (changelog says `1.0.0-1`).
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-1.0");
    copy_recursive(&fixture_root, &workspace_path);

    // Second capture: with a supplied expected_version
    // that disagrees with the staged changelog. The
    // runtime uses the supplied value as the candidate's
    // `source_version`; the source_preparation check
    // records the disagreement as an
    // `E_VERSION_MISMATCH` diagnostic.
    let _ = d
        .call(
            "candidate.capture",
            candidate_capture_args_with_version(&job_id, "example", Some("9.9.9-1")),
        )
        .await;

    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let check_id = next["actions"]["allowed"]
        .as_array()
        .and_then(|a| {
            a.iter().find_map(|a| {
                a.get("run_check")?
                    .get("check_id")?
                    .as_str()
                    .map(String::from)
            })
        })
        .expect("job.next_actions should list a run_check action for the active candidate");

    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": check_id.clone(),
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "check.run was not served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["error"],
        Value::Null,
        "check.run returned a protocol error: {body}"
    );
    let result = &body["result"]["structuredContent"];
    let evidence_id = result["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();

    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": {"evidence_id": evidence_id.clone()},
                },
            }),
            Some(TOKEN),
        )
        .await;
    let get_body: Value = get_response.json().await.expect("json body");
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array on the Evidence row");
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id string")
        .to_string();

    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    let bytes_base64 = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine as _;
    let artifact_bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64)
        .expect("base64-decode artifact bytes");
    let report: Value = serde_json::from_slice(&artifact_bytes).expect("report JSON");

    // The changelog parser still reports the real
    // fixture version (the source tree is unchanged).
    assert_eq!(
        report["findings"]["changelog_version"].as_str(),
        Some("1.0.0-1"),
        "the changelog parser must read the real fixture version"
    );
    // The cross-check is `false` because the
    // candidate's `package.version` is the supplied
    // `9.9.9-1`, not the changelog's `1.0.0-1`.
    assert_eq!(
        report["findings"]["changelog_version_matches_candidate"], false,
        "the supplied expected_version must override the changelog read; \
         the cross-check is then `false` because the values disagree: {report}"
    );
    let codes: Vec<&str> = report["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .map(|d| d["code"].as_str().expect("code string"))
        .collect();
    assert!(
        codes.contains(&"E_VERSION_MISMATCH"),
        "expected an E_VERSION_MISMATCH diagnostic; got {codes:?}"
    );
    assert_eq!(
        result["evidence_status"], "fail",
        "the cross-check failure is a `Verdict::Fail`, not `Pass`: {result}"
    );

    d.shutdown().await;
}

/// The check's evidence is bound to the captured tree, not
/// the live workspace. PHASE-1.1 PR 1.1.B (evidence
/// integrity): after `candidate.capture`, a concurrent
/// `workspace.apply_patch` would change the live workspace's
/// tree, but the check must still see the captured tree —
/// the runtime materialises a `git worktree add --detach`
/// snapshot at the candidate's tree OID and passes that
/// frozen path to the tool. The test stages the good
/// fixture, captures, then deletes the live workspace's
/// `debian/changelog`. With the GREEN code, the
/// `debian.inspect.source_preparation` check still passes
/// because it reads from the frozen worktree (which has
/// the original changelog). With the pre-1.1.B code, the
/// check would fail because the live workspace has no
/// changelog for the cross-check to read.
#[tokio::test]
async fn check_evidence_is_bound_to_the_captured_tree_not_the_live_workspace() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();

    // First capture: provision the workspace.
    let _ = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // Stage the good fixture.
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-1.0");
    copy_recursive(&fixture_root, &workspace_path);

    // Second capture: bind the candidate to the staged
    // fixture's tree.
    let _ = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // Now mutate the live workspace *after* capture, so
    // any check that reads the live workspace would see
    // the broken state. With the GREEN code, the check
    // materialises a frozen worktree from the candidate's
    // tree OID (which is the pre-mutation tree) and reads
    // from there, so the mutation is invisible to the
    // check.
    let live_changelog = workspace_path.join("debian").join("changelog");
    assert!(
        live_changelog.is_file(),
        "the staged fixture has a debian/changelog"
    );
    std::fs::remove_file(&live_changelog).expect("delete live changelog");
    assert!(
        !live_changelog.is_file(),
        "the live workspace's changelog is gone"
    );

    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let source_preparation_check_id = next["actions"]["allowed"]
        .as_array()
        .and_then(|a| {
            a.iter().find_map(|a| {
                a.get("run_check")?
                    .get("check_id")?
                    .as_str()
                    .map(String::from)
            })
        })
        .expect("job.next_actions should list a run_check action for the active candidate");

    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": source_preparation_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "check.run was not served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["error"],
        Value::Null,
        "check.run returned a protocol error: {body}"
    );
    let result = &body["result"]["structuredContent"];
    let evidence_id = result["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();
    let evidence_status = result["evidence_status"]
        .as_str()
        .expect("evidence_status string");
    assert_eq!(
        evidence_status, "pass",
        "the check sees the captured tree, not the live workspace: \
         the source_preparation check passes because the frozen worktree \
         has the original changelog. With the pre-1.1.B code the check \
         would read the live workspace (no changelog) and fail: {result}"
    );

    // The bound report carries the changelog version from
    // the captured tree, not from the live (now empty)
    // workspace. The cross-check passes because the
    // candidate's `package.version` (resolved from the
    // captured changelog by 1.1.A) matches the frozen
    // worktree's changelog version.
    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": {"evidence_id": evidence_id.clone()},
                },
            }),
            Some(TOKEN),
        )
        .await;
    let get_body: Value = get_response.json().await.expect("json body");
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array");
    assert!(!artifacts.is_empty(), "evidence has at least one artifact");
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id string")
        .to_string();
    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    let bytes_base64 = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine as _;
    let artifact_bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64)
        .expect("base64-decode artifact bytes");
    let report: Value = serde_json::from_slice(&artifact_bytes).expect("report JSON");
    assert_eq!(
        report["findings"]["changelog_version"].as_str(),
        Some("1.0.0-1"),
        "the changelog parser reads from the frozen worktree (the \
         captured tree), not from the live (now empty) workspace: {report}"
    );
    assert_eq!(
        report["findings"]["changelog_version_matches_candidate"], true,
        "the cross-check passes because the candidate's `package.version` \
         (resolved from the captured changelog) matches the frozen \
         worktree's changelog version: {report}"
    );

    d.shutdown().await;
}

/// The check fails when the candidate's identity
/// disagrees with the source tree's `debian/changelog`
/// first-entry source name. The §17 fail path is
/// distinct from the `infrastructure_error` path
/// exercised by the 1B.3 test above (no `debian/`
/// at all in the workspace).
#[tokio::test]
async fn debian_inspect_source_preparation_fails_on_a_mismatched_changelog() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    // Create a job, capture a candidate whose identity
    // says `example`, but stage the broken fixture whose
    // changelog says `broken-example`.
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let check_id =
        capture_against_staged_fixture(&mut d, &job_id, "example", "example-broken-1.0.0").await;

    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": check_id.clone(),
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "check.run was not served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["error"],
        Value::Null,
        "check.run returned a protocol error: {body}"
    );
    let result = &body["result"]["structuredContent"];
    assert_eq!(
        result["evidence_status"], "fail",
        "the broken fixture's changelog says `broken-example` while the \
         candidate says `example`; the cross-check fails and the verdict is \
         `fail` (not `infrastructure_error`, which is reserved for the tool \
         itself being unable to run): {result}"
    );
    assert_eq!(
        result["gate_status"], "fail",
        "a fail makes the gate fail (the agent cannot progress to the next stage): {result}"
    );

    d.shutdown().await;
}

/// 1C.2: the comprehensive `debian.inspect.source_analysis`
/// check returns the §18 `DebianSourceReportV1` on a
/// well-formed Debian source tree. The integration test
/// runs the check through MCP, then reads the bound
/// report artifact via `evidence.artifact.read` and
/// asserts the §18 comprehensive fields are populated.
///
/// In the 1C.2 RED commit, the report struct is the
/// minimal shape (§17 tri-state + candidate identity +
/// first-entry changelog block only); the §18
/// comprehensive fields are absent. The integration
/// test fails because the asserted fields are absent
/// from the report. In the 1C.2 GREEN commit, the
/// struct is the comprehensive shape and the test
/// passes.
#[tokio::test]
async fn debian_inspect_source_analysis_emits_the_comprehensive_section18_report() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    // 1. Job + candidate capture.
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let _captured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 2. Stage the good fixture into the auto-provisioned
    //    workspace handle, then capture a second time
    //    against the staged tree. The two-capture pattern
    //    is required because `capture_candidate` resolves
    //    the version from `debian/changelog` (PHASE-1.1
    //    PR 1.1.A), and the changelog is only present
    //    after the fixture is staged. The second capture
    //    is the candidate the rest of the test operates
    //    on; the first capture's placeholder candidate
    //    is overwritten by the runtime's
    //    `activate_candidate` semantics.
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(handles.len(), 1);
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-1.0");
    copy_recursive(&fixture_root, &workspace_path);
    let _recaptured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 3. `next_actions` returns the 5 pre-build
    //    `run_check` actions (1C.1, 1C.2, 1C.3, 1D.2,
    //    1E.2) planned against the staged tree. The
    //    first is the source-preparation check (1C.1);
    //    the second is the source-analysis check (1C.2)
    //    the test exercises.
    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .expect("allowed array");
    let check_ids: Vec<&str> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .collect();
    assert_eq!(
        check_ids.len(),
        5,
        "the production Debian adapter plans five checks (1C.1 + 1C.2 + 1C.3 + 1D.2 + 1E.2): {next}"
    );
    let source_preparation_check_id = check_ids[0].to_string();
    let source_analysis_check_id = check_ids[1].to_string();

    // 4. Run source_preparation (the 1C.1 check) first.
    //    This confirms the workspace path is set up
    //    correctly and the candidate identity is good.
    let prep_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": source_preparation_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(prep_response.status(), 200, "check.run was not served");
    let prep_body: Value = prep_response.json().await.expect("json body");
    assert_eq!(prep_body["error"], Value::Null, "{prep_body}");
    let prep_result = &prep_body["result"]["structuredContent"];
    assert_eq!(
        prep_result["evidence_status"], "pass",
        "the good fixture's §17 checks all hold; source_preparation passes: {prep_result}"
    );

    // 5. Run source_analysis (the 1C.2 check).
    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": source_analysis_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "check.run was not served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["error"],
        Value::Null,
        "check.run protocol error: {body}"
    );
    let result = &body["result"]["structuredContent"];
    assert_eq!(
        result["evidence_status"], "pass",
        "the good fixture's source tree is well-formed; the source_analysis tool's \
         verdict is `pass` and the normalizer maps it to EvidenceStatus::Pass: {result}"
    );
    assert_eq!(
        result["gate_status"], "pass",
        "a pass lets the gate open: {result}"
    );
    let evidence_id = result["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();

    // 6. Read the bound report artifact via
    //    `evidence.artifact.read` and assert the §18
    //    comprehensive fields are populated. In the 1C.2
    //    RED commit, the report is the minimal struct and
    //    these fields are absent; the test fails. In the
    //    1C.2 GREEN commit, the struct is comprehensive
    //    and the test passes.
    //
    //    The flow is: `evidence.get` returns the Evidence
    //    row with its bound `ArtifactRef` list; the first
    //    artifact's `id` is the `artifact_id` the read
    //    tool needs (the executor binds one report per
    //    check run); the read tool returns the artifact
    //    bytes base64-encoded.
    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": {
                        "evidence_id": evidence_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(get_response.status(), 200, "evidence.get was not served");
    let get_body: Value = get_response.json().await.expect("json body");
    assert_eq!(
        get_body["error"],
        Value::Null,
        "evidence.get protocol error: {get_body}"
    );
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array on the Evidence row");
    assert!(
        !artifacts.is_empty(),
        "the executor binds at least one report artifact per check.run: {get_body}"
    );
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id string")
        .to_string();

    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(
        artifact_response.status(),
        200,
        "evidence.artifact.read was not served"
    );
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    assert_eq!(
        artifact_body["error"],
        Value::Null,
        "evidence.artifact.read protocol error: {artifact_body}"
    );
    // The artifact body is the JSON the tool emitted on
    // stdout, base64-encoded by the read tool.
    let bytes_base64 = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine as _;
    let artifact_bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64)
        .expect("base64-decode artifact bytes");
    let report: Value = serde_json::from_slice(&artifact_bytes).expect("report JSON");

    let binary_packages = report["binary_packages"]
        .as_array()
        .expect("binary_packages array");
    assert!(
        !binary_packages.is_empty(),
        "the fixture's `debian/control` declares one binary package; \
         binary_packages must be non-empty: {report}"
    );
    let patches = report["patches"].as_array().expect("patches array");
    assert!(
        !patches.is_empty(),
        "the fixture's `debian/patches/series` references one patch; \
         patches must be non-empty: {report}"
    );
    let tests = report["tests"].as_array().expect("tests array");
    assert!(
        !tests.is_empty(),
        "the fixture's `debian/tests/control` declares one autopkgtest; \
         tests must be non-empty: {report}"
    );
    assert!(
        report["watch"].is_object(),
        "the fixture's `debian/watch` parses as a v4 watch object: {report}"
    );
    assert_eq!(
        report["rules"]["executable"].as_bool(),
        Some(true),
        "the fixture's `debian/rules` is executable: {report}"
    );
    assert_eq!(
        report["source_format"].as_str(),
        Some("3.0 (quilt)"),
        "the fixture's `debian/source/format` is `3.0 (quilt)`: {report}"
    );

    d.shutdown().await;
}

/// 1C.3: the comprehensive `debian.inspect.maintenance_context`
/// check returns the §26 `DebianMaintenanceContextV1` on a
/// well-formed Debian source tree. The integration test
/// runs the check through MCP, then reads the bound
/// report artifact via `evidence.artifact.read` and
/// asserts the §26 maintenance-dimension fields are
/// populated (patches, tests, watch, gbp, maintainer
/// scripts, upstream_relationship).
///
/// In the 1C.3 RED commit, the report struct is the
/// minimal shape (§17 tri-state + provenance only); the
/// §26 comprehensive fields are absent. The integration
/// test fails because the asserted fields are absent
/// from the report. In the 1C.3 GREEN commit, the
/// struct is the comprehensive shape and the test
/// passes.
#[tokio::test]
async fn debian_inspect_maintenance_context_emits_the_comprehensive_section19_report() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    // 1. Job + candidate capture.
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let _captured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 2. Stage the good fixture into the auto-provisioned
    //    workspace handle, then capture a second time
    //    against the staged tree. The two-capture pattern
    //    is required because `capture_candidate` resolves
    //    the version from `debian/changelog` (PHASE-1.1
    //    PR 1.1.A), and the changelog is only present
    //    after the fixture is staged. The second capture
    //    is the candidate the rest of the test operates
    //    on; the first capture's placeholder candidate
    //    is overwritten by the runtime's
    //    `activate_candidate` semantics.
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(handles.len(), 1);
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-1.0");
    copy_recursive(&fixture_root, &workspace_path);
    let _recaptured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 3. `next_actions` returns the 5 pre-build
    //    `run_check` actions (1C.1, 1C.2, 1C.3, 1D.2,
    //    1E.2) planned against the staged tree. The
    //    first is the source-preparation check (1C.1);
    //    the second is the source-analysis check (1C.2);
    //    the third is the maintenance-context check
    //    (1C.3) the test exercises.
    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .expect("allowed array");
    let check_ids: Vec<&str> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .collect();
    assert_eq!(
        check_ids.len(),
        5,
        "the production Debian adapter plans five checks (1C.1 + 1C.2 + 1C.3 + 1D.2 + 1E.2): {next}"
    );
    let source_preparation_check_id = check_ids[0].to_string();
    let source_analysis_check_id = check_ids[1].to_string();
    let maintenance_context_check_id = check_ids[2].to_string();

    // 4. Run source_preparation (the 1C.1 check) first.
    //    This confirms the workspace path is set up
    //    correctly and the candidate identity is good.
    let prep_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": source_preparation_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(prep_response.status(), 200, "check.run was not served");
    let prep_body: Value = prep_response.json().await.expect("json body");
    assert_eq!(prep_body["error"], Value::Null, "{prep_body}");
    let prep_result = &prep_body["result"]["structuredContent"];
    assert_eq!(
        prep_result["evidence_status"], "pass",
        "the good fixture's §17 checks all hold; source_preparation passes: {prep_result}"
    );

    // 5. Run source_analysis (the 1C.2 check).
    let analysis_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": source_analysis_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(analysis_response.status(), 200, "check.run was not served");
    let analysis_body: Value = analysis_response.json().await.expect("json body");
    assert_eq!(analysis_body["error"], Value::Null, "{analysis_body}");
    let analysis_result = &analysis_body["result"]["structuredContent"];
    assert_eq!(
        analysis_result["evidence_status"], "pass",
        "the good fixture's source tree is well-formed; the source_analysis tool's \
         verdict is `pass` and the normalizer maps it to EvidenceStatus::Pass: {analysis_result}"
    );

    // 6. Run maintenance_context (the 1C.3 check).
    let response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": maintenance_context_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(response.status(), 200, "check.run was not served");
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["error"],
        Value::Null,
        "check.run protocol error: {body}"
    );
    let result = &body["result"]["structuredContent"];
    assert_eq!(
        result["evidence_status"], "pass",
        "the good fixture's source tree is well-formed; the maintenance_context tool's \
         verdict is `pass` and the normalizer maps it to EvidenceStatus::Pass: {result}"
    );
    assert_eq!(
        result["gate_status"], "pass",
        "a pass lets the gate open: {result}"
    );
    let evidence_id = result["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();

    // 7. Read the bound report artifact via
    //    `evidence.artifact.read` and assert the §26
    //    maintenance-dimension fields are populated. In
    //    the 1C.3 RED commit, the report is the minimal
    //    struct and these fields are absent; the test
    //    fails. In the 1C.3 GREEN commit, the struct is
    //    comprehensive and the test passes.
    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": {
                        "evidence_id": evidence_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(get_response.status(), 200, "evidence.get was not served");
    let get_body: Value = get_response.json().await.expect("json body");
    assert_eq!(
        get_body["error"],
        Value::Null,
        "evidence.get protocol error: {get_body}"
    );
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array on the Evidence row");
    assert!(
        !artifacts.is_empty(),
        "the executor binds at least one report artifact per check.run: {get_body}"
    );
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id string")
        .to_string();

    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(
        artifact_response.status(),
        200,
        "evidence.artifact.read was not served"
    );
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    assert_eq!(
        artifact_body["error"],
        Value::Null,
        "evidence.artifact.read protocol error: {artifact_body}"
    );
    // The artifact body is the JSON the tool emitted on
    // stdout, base64-encoded by the read tool.
    let bytes_base64 = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine as _;
    let artifact_bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64)
        .expect("base64-decode artifact bytes");
    let report: Value = serde_json::from_slice(&artifact_bytes).expect("report JSON");

    // Assert the §26 maintenance-dimension fields are populated.
    let patches = report["patches"].as_array().expect("patches array");
    assert!(
        !patches.is_empty(),
        "the fixture's `debian/patches/series` references one patch; \
         patches must be non-empty: {report}"
    );
    let tests = report["tests"].as_array().expect("tests array");
    assert!(
        !tests.is_empty(),
        "the fixture's `debian/tests/control` declares one autopkgtest; \
         tests must be non-empty: {report}"
    );
    assert!(
        report["watch"].is_object(),
        "the fixture's `debian/watch` parses as a v4 watch object: {report}"
    );
    assert!(
        report["gbp"].is_object(),
        "the fixture's `debian/gbp.conf` is parsed into a gbp object: {report}"
    );
    assert!(
        report["maintainer_scripts"].is_object(),
        "the fixture's maintainer scripts are parsed into a maintainer_scripts object: {report}"
    );
    assert_eq!(
        report["maintainer_scripts"]["postinst"]["executable"].as_bool(),
        Some(true),
        "the fixture's `debian/postinst` is executable: {report}"
    );
    assert!(
        report["upstream_relationship"].is_object(),
        "the upstream_relationship sub-struct is present even with all-null fields: {report}"
    );

    d.shutdown().await;
}

/// 1E.2 — `debian.issue.bts_snapshot` is the §23 BTS
/// read-only transport + §24 tri-state verdict
/// normalizer. The hermetic mode of the binary's
/// `bts-snapshot` subcommand emits a hard-coded
/// `DebianIssueReportV1` with `verdict: pass`. This
/// integration test stages the §29 fixture, runs
/// the bts-snapshot check, and asserts the
/// evidence-bound report matches the §24 binary
/// tri-state (`verdict: pass`).
#[tokio::test(flavor = "current_thread")]
async fn debian_issue_bts_snapshot_passes_against_the_hermetic_fixture() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    // 1. Job + candidate capture.
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let _captured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 1. `next_actions` returns five `run_check`
    //    actions (1C.1 + 1C.2 + 1C.3 + 1D.2 + 1E.2).
    //    The 1E.2 check is the *fifth* — `bts-snapshot`
    //    is the 1E.2 routing key into
    //    `EvidenceKind::IssueCorrelation`.
    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .expect("allowed array");
    let check_ids: Vec<&str> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .collect();
    assert_eq!(check_ids.len(), 5);
    let bts_snapshot_check_id = check_ids[4].to_string();

    // 2. Run the bts-snapshot check. The hermetic
    //    mode of the binary's `bts-snapshot`
    //    subcommand emits a hard-coded
    //    `DebianIssueReportV1` with
    //    `verdict: pass` (regardless of bug count —
    //    §24 "A package having 50 open bugs is not a
    //    tool failure"). The fixture workspace is not
    //    read; the bts-snapshot subcommand is
    //    hermetic.
    let run_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": bts_snapshot_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(run_response.status(), 200, "check.run was not served");
    let run_body: Value = run_response.json().await.expect("json body");
    assert_eq!(
        run_body["error"],
        Value::Null,
        "check.run protocol error: {run_body}"
    );

    // 3. Read the evidence row + report artifact.
    let evidence_id = run_body["result"]["structuredContent"]["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();
    assert_eq!(
        run_body["result"]["structuredContent"]["evidence_status"].as_str(),
        Some("pass"),
        "the §24 hermetic `pass` verdict maps to `EvidenceStatus::Pass` (1A.3): {run_body}"
    );

    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": { "evidence_id": evidence_id },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(get_response.status(), 200, "evidence.get was not served");
    let get_body: Value = get_response.json().await.expect("json body");
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array");
    assert!(
        !artifacts.is_empty(),
        "the executor binds at least one report artifact per check.run: {get_body}"
    );
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id string")
        .to_string();

    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    let bytes_base64 = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine as _;
    let artifact_bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64)
        .expect("base64-decode artifact bytes");
    let report: Value = serde_json::from_slice(&artifact_bytes).expect("report JSON");

    // 4. The wire format is the §23 / §24 discriminator
    //    + binary tri-state (no `Fail` per §24). The
    //    GREEN-state report carries the §23
    //    comprehensive fields (`source_package`,
    //    `snapshot_at`, `bugs`, `summary`).
    assert_eq!(
        report["report_kind"].as_str(),
        Some("debian.issue.bts_snapshot_v1"),
        "the §24 wire-format discriminator: {report}"
    );
    assert_eq!(
        report["schema_version"].as_u64(),
        Some(1),
        "schema_version 1: {report}"
    );
    assert_eq!(
        report["verdict"].as_str(),
        Some("pass"),
        "the §24 hermetic `pass` verdict is binary — no `Fail` case for the BTS snapshot: {report}"
    );

    // 5. The §23 comprehensive fields are populated
    //    by the hermetic fixture (3 bugs, summary with
    //    `open: 2, serious: 1, patch_tagged: 1,
    //    forwarded: 1`).
    assert_eq!(
        report["source_package"].as_str(),
        Some("example"),
        "the §23 source_package is the candidate's `package.name`: {report}"
    );
    assert!(
        report["snapshot_at"].is_string(),
        "the §23 snapshot_at is a non-empty RFC3339 UTC timestamp: {report}"
    );
    let bugs = report["bugs"].as_array().expect("§23 `bugs` is an array");
    assert_eq!(
        bugs.len(),
        3,
        "the §23 hermetic fixture has 3 bugs (one open/serious, one open/forwarded, one done): {report}"
    );
    let summary = &report["summary"];
    assert_eq!(
        summary["open"].as_u64(),
        Some(2),
        "the §23 summary `open` count is 2: {report}"
    );
    assert_eq!(
        summary["serious"].as_u64(),
        Some(1),
        "the §23 summary `serious` count is 1: {report}"
    );
    assert_eq!(
        summary["patch_tagged"].as_u64(),
        Some(1),
        "the §23 summary `patch_tagged` count is 1: {report}"
    );
    assert_eq!(
        summary["forwarded"].as_u64(),
        Some(1),
        "the §23 summary `forwarded` count is 1: {report}"
    );

    d.shutdown().await;
}

/// 1D.2 integration test: a candidate whose
/// `debian/control` declares a `Standards-Version`
/// above the §19 manifest's `standards_version_target`
/// (= `"4.7.4"`) yields a `Verdict::InfrastructureError`
/// evidence with the `E_POLICY_BASELINE_STALE` diagnostic
/// and the `AboveBaseline` comparison outcome. The test
/// stages the `example-future-1.0` hermetic fixture
/// (its `Standards-Version: 4.9.0` is above the
/// manifest baseline) and exercises the full MCP
/// capture → plan → check.run → evidence.get →
/// evidence.artifact.read → decode path.
///
/// PHASE-1.1 PR 1.1.D (policy baseline correctness):
/// the pre-1.1.D test expected `Verdict::Fail`
/// (the candidate is bad) and `baseline_policy_version`
/// (the *Policy Manual document* version, 4.7.4.1).
/// The 1.1.D-corrected test expects
/// `Verdict::InfrastructureError` (the manifest is
/// stale, not the package — the spec at line 978
/// is explicit that an above-baseline candidate
/// "must not be marked bad") and
/// `standards_version_target` (= `"4.7.4"`, the
/// `Standards-Version` target).
#[tokio::test]
async fn debian_policy_delta_review_blocks_a_candidate_above_the_baseline() {
    let tmp = Arc::new(tempfile::tempdir().expect("tmpdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let _captured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // Stage the future-version fixture into the
    // auto-provisioned workspace handle, then capture
    // a second time so the candidate's `source_version`
    // is bound to the staged tree (PHASE-1.1 PR 1.1.A:
    // `capture_candidate` resolves the version from
    // `debian/changelog`). The `Standards-Version: 4.9.0`
    // is above the manifest's `4.7.4.1` baseline.
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(handles.len(), 1);
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-future-1.0");
    copy_recursive(&fixture_root, &workspace_path);
    let _recaptured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .expect("allowed array");
    let check_ids: Vec<String> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .map(|s| s.to_string())
        .collect();
    // The 1D.2 policy-delta check is the 4th entry
    // in the inspection plan (after source_preparation,
    // source_analysis, and maintenance_context).
    let policy_delta_check_id = check_ids
        .get(3)
        .cloned()
        .expect("policy-delta check_id must be in next_actions");

    // Run only the policy-delta check (skip the
    // 1C.1 / 1C.2 / 1C.3 checks; the future
    // fixture lacks patches/, watch, etc., and
    // those checks would fail for unrelated
    // reasons). The 1D.2 check is the
    // load-bearing gate; the others are
    // re-verified by the `…_passes_a_candidate_below_the_baseline`
    // test below.
    let _ = check_ids;
    let run_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": policy_delta_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(run_response.status(), 200, "check.run was not served");
    let run_body: Value = run_response.json().await.expect("json body");
    assert_eq!(run_body["error"], Value::Null, "{run_body}");
    let run_result = &run_body["result"]["structuredContent"];
    assert_eq!(
        run_result["evidence_status"], "infrastructure_error",
        "the §22 gate must fire on Standards-Version 4.9.0 (above baseline 4.7.4): {run_result}"
    );
    let evidence_id = run_result["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();

    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": { "evidence_id": evidence_id },
                },
            }),
            Some(TOKEN),
        )
        .await;
    let get_body: Value = get_response.json().await.expect("json body");
    assert_eq!(get_body["error"], Value::Null, "{get_body}");
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array");
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id")
        .to_string();

    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    let encoded = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("base64 decode");
    let report: serde_json::Value = serde_json::from_slice(&decoded).expect("report JSON");

    assert_eq!(
        report["verdict"].as_str(),
        Some("infrastructure_error"),
        "the future-fixture verdict is infrastructure_error (post-1.1.D): {report}"
    );
    assert_eq!(
        report["candidate_standards_version"].as_str(),
        Some("4.9.0"),
        "the candidate's Standards-Version is 4.9.0: {report}"
    );
    assert_eq!(
        report["baseline_standards_version"].as_str(),
        Some("4.7.4"),
        "the manifest's standards_version_target is 4.7.4 (post-1.1.D): {report}"
    );
    assert_eq!(
        report["comparison_outcome"].as_str(),
        Some("above_baseline"),
        "the §22 comparison outcome is above_baseline: {report}"
    );
    let diags = report["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags
            .iter()
            .any(|d| d["code"].as_str() == Some("E_POLICY_BASELINE_STALE")),
        "the §22 E_POLICY_BASELINE_STALE diagnostic is in the report: {report}"
    );

    d.shutdown().await;
}

/// 1D.2 integration test: the good fixture's
/// `Standards-Version: 4.6.2` is below the
/// manifest's `4.7.4` baseline, so the check
/// yields `Verdict::Pass` with the
/// `BelowBaseline` comparison outcome. This
/// is the §22 pass-path proof: the gate does
/// not over-fire on known-policy candidates.
#[tokio::test]
async fn debian_policy_delta_review_passes_a_candidate_below_the_baseline() {
    let tmp = Arc::new(tempfile::tempdir().expect("tmpdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let _captured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // Stage the good fixture into the auto-provisioned
    // workspace handle, then capture a second time so
    // the candidate's `source_version` is bound to the
    // staged tree (PHASE-1.1 PR 1.1.A: `capture_candidate`
    // resolves the version from `debian/changelog`).
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(handles.len(), 1);
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-1.0");
    copy_recursive(&fixture_root, &workspace_path);
    let _recaptured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .expect("allowed array");
    let check_ids: Vec<String> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .map(|s| s.to_string())
        .collect();
    // The 1D.2 policy-delta check is the 4th entry
    // in the inspection plan (after source_preparation,
    // source_analysis, and maintenance_context).
    let policy_delta_check_id = check_ids
        .get(3)
        .cloned()
        .expect("policy-delta check_id must be in next_actions");

    let run_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check.run",
                    "arguments": {
                        "job_id": job_id,
                        "check_id": policy_delta_check_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    assert_eq!(run_response.status(), 200, "check.run was not served");
    let run_body: Value = run_response.json().await.expect("json body");
    assert_eq!(run_body["error"], Value::Null, "{run_body}");
    let run_result = &run_body["result"]["structuredContent"];
    assert_eq!(
        run_result["evidence_status"], "pass",
        "the §22 gate does not fire on Standards-Version 4.6.2 (below baseline 4.7.4): {run_result}"
    );
    let evidence_id = run_result["evidence_id"]
        .as_str()
        .expect("evidence_id string")
        .to_string();

    let get_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "evidence.get",
                    "arguments": { "evidence_id": evidence_id },
                },
            }),
            Some(TOKEN),
        )
        .await;
    let get_body: Value = get_response.json().await.expect("json body");
    let artifacts = get_body["result"]["structuredContent"]["evidence"]["artifacts"]
        .as_array()
        .expect("artifacts array");
    let artifact_id = artifacts[0]["id"]
        .as_str()
        .expect("artifact id")
        .to_string();

    let artifact_response = d
        .post(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "evidence.artifact.read",
                    "arguments": {
                        "evidence_id": evidence_id,
                        "artifact_id": artifact_id,
                    },
                },
            }),
            Some(TOKEN),
        )
        .await;
    let artifact_body: Value = artifact_response.json().await.expect("json body");
    let encoded = artifact_body["result"]["structuredContent"]["bytes_base64"]
        .as_str()
        .expect("bytes_base64 string");
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("base64 decode");
    let report: serde_json::Value = serde_json::from_slice(&decoded).expect("report JSON");

    assert_eq!(
        report["verdict"].as_str(),
        Some("pass"),
        "the good-fixture verdict is pass: {report}"
    );
    assert_eq!(
        report["candidate_standards_version"].as_str(),
        Some("4.6.2"),
        "the candidate's Standards-Version is 4.6.2: {report}"
    );
    assert_eq!(
        report["comparison_outcome"].as_str(),
        Some("below_baseline"),
        "the §22 comparison outcome is below_baseline: {report}"
    );

    d.shutdown().await;
}

/// 1F.1 — PHASE-1.md §14 / §36 acceptance test: the
/// existing state machine walks a real Debian fixture
/// from `EventDetected` to `SourceIntegrity` end-to-end
/// through MCP and then *stops*, because the
/// `BuildValidation` gate is not satisfied.
///
/// Five pre-build checks gate the first five
/// transitions (PHASE-1.md §13):
/// - `debian.inspect.source_preparation` →
///   `SourcePreparation` gate (rule 0)
/// - `debian.inspect.source_analysis` →
///   `SourceAnalysis` gate (rule 1)
/// - `debian.issue.bts_snapshot` →
///   `IssueAnalysis` gate (rule 2)
/// - `debian.inspect.maintenance_context` →
///   `Maintenance` gate (rule 3)
/// - `debian.policy.delta_review` →
///   `PolicyEvaluation` gate (rule 4)
///
/// Rule 5 (`SourceIntegrity → BuildValidation`) requires
/// a `BuildValidation` gate no Phase 1 check satisfies.
/// The state machine must therefore return
/// `NoOp{current: SourceIntegrity}` on the next
/// reconcile call — the architectural property §14
/// mandates.
///
/// The RED→GREEN pair (PHASE-1.md §36 / EXECUTION-PLAN
/// §4): the RED commit omits the `bts_snapshot` check
/// (which gates rule 2), so reconcile #3 returns
/// `Blocked{current: SourceReview, target: CandidateAssembly}`
/// instead of advancing. The GREEN commit adds the
/// `bts_snapshot` check so all five gates are satisfied
/// and the walker advances through all five rules.
///
/// Test name deliberately includes the full claim so
/// the broken-then-fixed signal is visible from `cargo
/// test` alone.
#[tokio::test]
async fn debian_state_machine_vertical_slice_walks_event_detected_to_source_integrity_and_stops_at_the_missing_build_validation_gate()
 {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    // 1. job.create + candidate.capture for the `example`
    //    Debian package. The good fixture's identity
    //    matches.
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let _captured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 2. Stage the good fixture into the job's
    //    workspace handle, then capture a second time
    //    so the candidate's `source_version` is bound
    //    to the staged tree. The two-capture pattern
    //    is required because `capture_candidate`
    //    resolves the version from `debian/changelog`
    //    (PHASE-1.1 PR 1.1.A), and the changelog is
    //    only present after the fixture is staged.
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(handles.len(), 1, "expected exactly one workspace handle");
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-1.0");
    copy_recursive(&fixture_root, &workspace_path);
    let _recaptured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 3. Discover the five pre-build check_ids via
    //    `job.next_actions`. The inspection plan emits
    //    them in the same order the production
    //    `DebianInspection::inspection_plan` builds
    //    them (1C.1, 1C.2, 1C.3, 1D.2, 1E.2).
    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .expect("allowed array");
    let check_ids: Vec<String> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .map(String::from)
        .collect();
    assert_eq!(
        check_ids.len(),
        5,
        "the production Debian inspection plan must plan five pre-build checks: {check_ids:?}"
    );

    // 4. Run each of the five checks. Each emits a
    //    `Verdict::Pass` against the good fixture;
    //    the normalizer maps `pass` to
    //    `EvidenceStatus::Pass` per §17. The
    //    `evidence_status` assertion is the runtime
    //    proof that the binary wrote and the normalizer
    //    classified the report correctly.
    //
    //    GREEN-COMMIT-ONLY: the `bts_snapshot` check
    //    is the line that flips this test from RED to
    //    GREEN. In RED, omitting it leaves the
    //    `IssueAnalysis` gate (rule 2) unsatisfied,
    //    and reconcile #3 returns `Blocked` instead of
    //    `Advanced`.
    for (label, check_id) in [
        ("source_preparation", &check_ids[0]),
        ("source_analysis", &check_ids[1]),
        ("maintenance_context", &check_ids[2]),
        ("policy_delta_review", &check_ids[3]),
        ("bts_snapshot", &check_ids[4]),
    ] {
        let run_result = d
            .call("check.run", json!({"job_id": job_id, "check_id": check_id}))
            .await;
        assert_eq!(
            run_result["evidence_status"], "pass",
            "the {label} check must pass against the good fixture: {run_result}"
        );
    }

    // 5. Walk the state machine. The `reconcile()`
    //    walker at service.rs:1533 advances through ALL
    //    satisfied rules in a single call (§41: the loop
    //    "continues through multiple trivially-satisfied
    //    stages until reaching a state requiring [agent
    //    action]") and returns the first stop reason.
    //
    //    With all five pre-build checks passing, the
    //    walker advances rules 0..=4 and stops on rule 5
    //    (SourceIntegrity → BuildValidation) which has no
    //    gate definition (no Phase 1 check produces
    //    BuildValidation evidence). The returned outcome
    //    is therefore `Blocked{current: source_integrity,
    //    target: build_validation}` — the state machine
    //    *did* walk from EventDetected to SourceIntegrity
    //    (the `current` field proves it), and it *did*
    //    stop at the right point (the `target` field
    //    proves the next-blocked transition is the one
    //    §14 names).
    //
    //    In the RED state (bts_snapshot omitted), the
    //    `IssueAnalysis` gate (rule 2) is unsatisfied, so
    //    the walker stops earlier — at `current:
    //    source_review, target: candidate_assembly` — and
    //    the assertion below fails. The test name is
    //    intentionally explicit so the failure message
    //    shows the architectural property the GREEN state
    //    demonstrates.
    let reconcile = d.call("job.reconcile", json!({"job_id": job_id})).await;
    let outcome = &reconcile["outcome"];
    assert_eq!(
        outcome["blocked"]["current"].as_str(),
        Some("source_integrity"),
        "the walker must reach SourceIntegrity before blocking: {reconcile}"
    );
    assert_eq!(
        outcome["blocked"]["target"].as_str(),
        Some("build_validation"),
        "the walker must stop at the §14 BuildValidation boundary: {reconcile}"
    );
    let blockers = outcome["blocked"]["blockers"]
        .as_array()
        .expect("blockers array");
    assert!(
        !blockers.is_empty(),
        "the BuildValidation gate must be reported as missing: {reconcile}"
    );
    let first_blocker = blockers[0].as_str().expect("blocker is a string");
    assert!(
        first_blocker.contains("MissingGate") || first_blocker.contains("build_validation"),
        "the §14 blocker names the BuildValidation gate: {first_blocker:?}"
    );

    // 6. The state machine stays at SourceIntegrity
    //    on subsequent reconcile calls. The state
    //    does not advance (BuildValidation is still
    //    unsatisfied) and does not regress. This is
    //    the structural catch: if a future PR ever
    //    registers a BuildValidation evidence row, this
    //    second call would no longer report the same
    //    Blocked outcome and the regression would be
    //    visible at the §97 gate.
    let second_reconcile = d.call("job.reconcile", json!({"job_id": job_id})).await;
    let second_outcome = &second_reconcile["outcome"];
    assert_eq!(
        second_outcome["blocked"]["current"].as_str(),
        Some("source_integrity"),
        "the state machine must remain at SourceIntegrity on a second reconcile: {second_reconcile}"
    );
    assert_eq!(
        second_outcome["blocked"]["target"].as_str(),
        Some("build_validation"),
        "the §14 BuildValidation boundary must remain blocked: {second_reconcile}"
    );

    d.shutdown().await;
}

/// `doc/phases/PHASE-1.md` §37 + §43 — maintainer intake
/// scenario.
///
/// Walks the five pre-build gates against the hermetic
/// fixture, reads every bound report artifact through the
/// `evidence.*` tool surface, and assembles a §43-shaped
/// intake summary. Every line in the summary must be
/// traceable to a specific evidence row's structured
/// report — not to the test's memory of what the fixture
/// looks like. The state machine then reconciles to
/// `SourceIntegrity` and stops at the `BuildValidation`
/// boundary (§14), confirming the workflow line of the
/// summary.
///
/// RED state: the §43 lines that depend on per-check
/// evidence (the structured report content) are absent
/// or wrong because the artifact read has not yet
/// happened in the loop — the summary is built from
/// `evidence_id` strings alone, with no field-level
/// claims. GREEN state: the summary is built by reading
/// the artifact and asserting on its parsed JSON.
#[tokio::test]
async fn maintainer_intake_scenario_assembles_a_43_shaped_exit_artifact_from_trusted_evidence() {
    let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
    let mut d = Daemon::start_on(Arc::clone(&tmp)).await;

    // 1. job.create + candidate.capture for `example`.
    let created = d.call("job.create", create_job_args()).await;
    let job_id = created["job_id"]
        .as_str()
        .expect("job_id string")
        .to_string();
    let _captured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 2. Stage the good fixture into the job's workspace
    //    handle, then capture a second time so the
    //    candidate's `source_version` is bound to the
    //    staged tree (PHASE-1.1 PR 1.1.A: `capture_candidate`
    //    resolves the version from `debian/changelog`).
    let workspaces_root = d.workspaces_root();
    let mut handles = std::fs::read_dir(&workspaces_root)
        .expect("read_dir workspaces")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(handles.len(), 1, "expected exactly one workspace handle");
    let workspace_path = handles.pop().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../containers/fixtures/debian/example-1.0");
    copy_recursive(&fixture_root, &workspace_path);
    let _recaptured = d
        .call(
            "candidate.capture",
            candidate_capture_args(&job_id, "example"),
        )
        .await;

    // 3. Discover the five pre-build check_ids via
    //    `job.next_actions`, in the same order the
    //    production `DebianInspection::inspection_plan`
    //    emits them.
    let next = d.call("job.next_actions", json!({"job_id": job_id})).await;
    let allowed = next["actions"]["allowed"]
        .as_array()
        .expect("allowed array");
    let check_ids: Vec<String> = allowed
        .iter()
        .filter_map(|a| a.get("run_check")?.get("check_id")?.as_str())
        .map(String::from)
        .collect();
    assert_eq!(
        check_ids.len(),
        5,
        "the production Debian inspection plan must plan five pre-build checks: {check_ids:?}"
    );

    // 4. Run each of the five checks, collect each
    //    `evidence_id` the runtime hands back. The
    //    `evidence_status == "pass"` assertion is the
    //    §17 trip-wire; the hermetic fixture passes
    //    all five (1C.1/1C.2/1C.3/1D.2/1E.2).
    let mut evidence_ids: Vec<(String, String)> = Vec::new();
    for (label, check_id) in [
        ("source_preparation", &check_ids[0]),
        ("source_analysis", &check_ids[1]),
        ("maintenance_context", &check_ids[2]),
        ("policy_delta_review", &check_ids[3]),
        ("bts_snapshot", &check_ids[4]),
    ] {
        let run_result = d
            .call("check.run", json!({"job_id": job_id, "check_id": check_id}))
            .await;
        assert_eq!(
            run_result["evidence_status"], "pass",
            "the {label} check must pass against the good fixture: {run_result}"
        );
        let evidence_id = run_result["evidence_id"]
            .as_str()
            .expect("evidence_id string")
            .to_string();
        evidence_ids.push((label.to_string(), evidence_id));
    }

    // 5. For each evidence_id, walk the `evidence.*`
    //    surface end-to-end. The 1A.4 contract is
    //    "read-only access to the durable evidence
    //    substrate"; the §37 contract is "produce a
    //    summary based only on trusted evidence
    //    artifacts." Every line in the §43-shaped
    //    summary below is sourced from one of these
    //    three calls — `evidence.list` enumerates the
    //    rows, `evidence.get` reads the row's
    //    producer/artifact binding, and
    //    `evidence.artifact.read` returns the §18 /
    //    §22 / §23 / §26 structured report.
    use base64::Engine as _;
    let mut report_by_label: std::collections::BTreeMap<String, Value> =
        std::collections::BTreeMap::new();
    let mut candidate_fingerprint: Option<String> = None;

    for (label, evidence_id) in &evidence_ids {
        // 5a. evidence.list — confirm the row is
        //     visible through the job-scoped list
        //     (the §31 read-only access surface).
        let list_result = d
            .call(
                "evidence.list",
                json!({ "job_id": job_id, "candidate_fingerprint": Value::Null }),
            )
            .await;
        let listed: Vec<&str> = list_result["rows"]
            .as_array()
            .expect("rows array")
            .iter()
            .filter_map(|row| row.get("id")?.as_str())
            .collect();
        assert!(
            listed.contains(&evidence_id.as_str()),
            "evidence.list must include the {label} evidence row {evidence_id}: {list_result}"
        );

        // 5b. evidence.get — read the row, capture
        //     the candidate fingerprint, and pull the
        //     first bound artifact id.
        let get_result = d
            .call("evidence.get", json!({ "evidence_id": evidence_id }))
            .await;
        if candidate_fingerprint.is_none() {
            candidate_fingerprint = Some(
                get_result["evidence"]["candidate"]
                    .as_str()
                    .expect("candidate fingerprint string")
                    .to_string(),
            );
        }
        let artifacts = get_result["evidence"]["artifacts"]
            .as_array()
            .expect("artifacts array");
        assert!(
            !artifacts.is_empty(),
            "the {label} check must bind at least one report artifact: {get_result}"
        );
        let artifact_id = artifacts[0]["id"]
            .as_str()
            .expect("artifact id string")
            .to_string();

        // 5c. evidence.artifact.read — read the
        //     structured report the binary emitted.
        let read_result = d
            .call(
                "evidence.artifact.read",
                json!({
                    "evidence_id": evidence_id,
                    "artifact_id": artifact_id,
                }),
            )
            .await;
        let bytes_base64 = read_result["bytes_base64"]
            .as_str()
            .expect("bytes_base64 string");
        let artifact_bytes = base64::engine::general_purpose::STANDARD
            .decode(bytes_base64)
            .expect("base64-decode artifact bytes");
        let report: Value = serde_json::from_slice(&artifact_bytes)
            .unwrap_or_else(|e| panic!("{label} report is not valid JSON: {e}"));
        report_by_label.insert(label.clone(), report);
    }

    // 6. Build the §43-shaped intake summary. Every
    //    field is sourced from one of the parsed
    //    reports — nothing is hand-written from the
    //    test's memory of the fixture.
    let source_analysis = report_by_label
        .get("source_analysis")
        .expect("source_analysis report must have been read");
    let maintenance_context = report_by_label
        .get("maintenance_context")
        .expect("maintenance_context report must have been read");
    let bts_snapshot = report_by_label
        .get("bts_snapshot")
        .expect("bts_snapshot report must have been read");
    let policy_delta = report_by_label
        .get("policy_delta_review")
        .expect("policy_delta_review report must have been read");

    let identity = &source_analysis["identity"];
    let source_metadata = &source_analysis["source_metadata"];
    let build_metadata = &source_analysis["build_metadata"];
    let binary_packages: Vec<&str> = source_analysis["binary_packages"]
        .as_array()
        .expect("binary_packages array")
        .iter()
        .filter_map(|p| p.get("name")?.as_str())
        .collect();
    let patches_count = maintenance_context["patches"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let tests_count = maintenance_context["tests"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let watch_present = maintenance_context
        .get("watch")
        .map(|w| !w.is_null())
        .unwrap_or(false);

    let intake = json!({
        "debian_package": identity.get("source").cloned().unwrap_or(Value::Null),
        "candidate": candidate_fingerprint
            .clone()
            .map(Value::from)
            .unwrap_or(Value::Null),
        "source_version": identity.get("version").cloned().unwrap_or(Value::Null),
        "source_format": source_analysis.get("source_format").cloned().unwrap_or(Value::Null),
        "binary_packages": binary_packages,
        "packaging": {
            "build_depends": build_metadata.get("build_depends").cloned().unwrap_or(Value::Null),
            "standards_version": source_metadata.get("standards_version").cloned().unwrap_or(Value::Null),
            "rules_requires_root": source_metadata.get("rules_requires_root").cloned().unwrap_or(Value::Null),
        },
        "patches": patches_count,
        "tests": tests_count,
        "upstream_monitoring": { "watch_present": watch_present },
        "bts": {
            "verdict": bts_snapshot.get("verdict").cloned().unwrap_or(Value::Null),
            "summary": bts_snapshot.get("summary").cloned().unwrap_or(Value::Null),
        },
        "policy": {
            "baseline": policy_delta.get("baseline_standards_version").cloned().unwrap_or(Value::Null),
            "declared": policy_delta.get("candidate_standards_version").cloned().unwrap_or(Value::Null),
            "delta": policy_delta.get("comparison_outcome").cloned().unwrap_or(Value::Null),
        },
    });

    // 7. Assert the §43 shape against the parsed
    //    reports. The §43 invariant: every substantive
    //    statement in the report must trace to a
    //    candidate / check / evidence / artifact /
    //    producer / authority chain. Here we walk
    //    the same six links the §43 spec lists and
    //    assert each one resolves to a real value
    //    (not Null) sourced from the reports.
    assert_eq!(
        intake["debian_package"], "example",
        "§43 line `Debian package` traces to source_analysis.identity.source: {intake}"
    );
    assert!(
        intake["candidate"].as_str().is_some(),
        "§43 line `Candidate` traces to the active candidate fingerprint: {intake}"
    );
    assert!(
        intake["source_version"].as_str().is_some(),
        "§43 line `Source version` traces to source_analysis.identity.version: {intake}"
    );
    assert_eq!(
        intake["source_format"], "3.0 (quilt)",
        "§43 line `Source format` traces to source_analysis.source_format: {intake}"
    );
    assert!(
        !binary_packages.is_empty(),
        "§43 line `Binary packages` traces to source_analysis.binary_packages[].name: {intake}"
    );
    assert!(
        intake["packaging"]["build_depends"].as_array().is_some(),
        "§43 line `Packaging` traces to source_analysis.source_metadata.build_depends: {intake}"
    );
    assert!(
        intake["packaging"]["standards_version"].as_str().is_some(),
        "§43 line `Packaging.Standards-Version` traces to source_analysis.source_metadata.standards_version: {intake}"
    );
    assert_eq!(
        intake["upstream_monitoring"]["watch_present"], true,
        "§43 line `Upstream monitoring` traces to maintenance_context.watch: {intake}"
    );
    assert_eq!(
        intake["bts"]["verdict"], "pass",
        "§43 line `BTS.verdict` traces to bts_snapshot.verdict: {intake}"
    );
    assert!(
        intake["policy"]["baseline"].as_str().is_some(),
        "§43 line `Policy.baseline` traces to policy_delta_review.baseline_policy_version: {intake}"
    );
    assert!(
        intake["policy"]["declared"].as_str().is_some(),
        "§43 line `Policy.declared` traces to policy_delta_review.declared_standards_version: {intake}"
    );
    assert!(
        intake["policy"]["delta"].is_string(),
        "§43 line `Policy.delta` traces to policy_delta_review.delta: {intake}"
    );

    // 8. The workflow line is sourced from the state
    //    machine's `Blocked` outcome at the
    //    `BuildValidation` boundary — the §14
    //    architectural fact the skill has to teach
    //    the agent, not a value the LLM remembers.
    let reconcile = d.call("job.reconcile", json!({"job_id": job_id})).await;
    let outcome = &reconcile["outcome"];
    let workflow = json!({
        "read_only_intake": "complete",
        "next_unimplemented_stage": outcome["blocked"]["target"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
    });
    assert_eq!(
        workflow["next_unimplemented_stage"], "build_validation",
        "§43 line `Workflow.next` traces to the §14 BuildValidation boundary, \
         not to LLM memory: {reconcile}"
    );
    assert_eq!(
        outcome["blocked"]["current"].as_str(),
        Some("source_integrity"),
        "the §43 `Workflow` line's source state is SourceIntegrity, not anything earlier: {reconcile}"
    );

    d.shutdown().await;
}
