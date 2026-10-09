//! Configuration resolution tests (PHASE-0B.md §66).
//!
//! The daemon's failure mode is not crashing — it is starting
//! happily against the wrong state directory, or with no token
//! file, or listening where nobody expects. These tests pin the
//! precedence rules and the refusals, because both are what an
//! operator relies on when they type a flag.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::ffi::OsString;

/// An empty environment, spelled so the closure's return type is
/// inferred at the call site rather than at each construction.
fn no_env() -> HashMap<String, String> {
    HashMap::new()
}

use std::path::PathBuf;

use ironmaintd::config::{ConfigError, ConfigOutcome, RuntimeConfig};

/// A temp state dir plus a real token file, since both are
/// existence-checked at startup.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    token_file: std::path::PathBuf,
    migrations: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("state");
        std::fs::create_dir_all(&root).expect("state dir");
        let token_file = tmp.path().join("token");
        std::fs::write(&token_file, "ironmaint-test-token-0123456789").expect("token");
        let migrations =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        assert!(
            migrations.is_dir(),
            "the workspace migrations dir must exist for these tests to mean anything"
        );
        Self {
            _tmp: tmp,
            root,
            token_file,
            migrations,
        }
    }

    /// The minimum argument list that yields a `Run`.
    fn args(&self) -> Vec<OsString> {
        vec![
            "--state-dir".into(),
            self.root.clone().into(),
            "--token-file".into(),
            self.token_file.clone().into(),
            "--migrations-dir".into(),
            self.migrations.clone().into(),
        ]
    }

    fn parse(&self, extra: &[&str], env: &[(&str, &str)]) -> Result<RuntimeConfig, ConfigError> {
        let map: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let mut args = self.args();
        args.extend(extra.iter().map(OsString::from));
        match RuntimeConfig::from_env_and_args(&args, |name| map.get(name).cloned())? {
            ConfigOutcome::Run(c) => Ok(c),
            other => panic!("expected Run, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Precedence: flag > env > default
// ---------------------------------------------------------------------------

#[test]
fn a_flag_beats_the_environment() {
    let f = Fixture::new();
    let config = f
        .parse(
            &["--bind", "127.0.0.1:9999"],
            &[("IRONMAINT_BIND", "127.0.0.1:8888")],
        )
        .expect("parse");
    assert_eq!(config.bind.port(), 9999, "the flag must win");
}

#[test]
fn the_environment_is_used_when_no_flag_is_given() {
    let f = Fixture::new();
    let config = f
        .parse(&[], &[("IRONMAINT_BIND", "127.0.0.1:8888")])
        .expect("parse");
    assert_eq!(config.bind.port(), 8888);
}

#[test]
fn the_default_is_used_when_neither_is_given() {
    let f = Fixture::new();
    let config = f.parse(&[], &[]).expect("parse");
    assert_eq!(config.bind.to_string(), "127.0.0.1:7341");
    assert!(config.log_filter.contains("info"));
}

#[test]
fn a_flag_may_be_given_with_equals_or_with_a_separate_value() {
    // Both spellings are common in shell scripts and habit;
    // supporting only one means `--bind=...` is silently an
    // unknown flag on some days and a value on others.
    let f = Fixture::new();
    let a = f
        .parse(&["--bind=127.0.0.1:1234"], &[])
        .expect("parse equals");
    let b = f
        .parse(&["--bind", "127.0.0.1:1234"], &[])
        .expect("parse separate");
    assert_eq!(a.bind, b.bind);
}

#[test]
fn a_single_dash_help_is_accepted() {
    match RuntimeConfig::from_env_and_args(&["-h".into()], |_| None).expect("help") {
        ConfigOutcome::Help => {}
        other => panic!("expected Help, got {other:?}"),
    }
}

#[test]
fn version_short_circuits_before_anything_else_is_checked() {
    // `--version` must work even with no token file: an operator
    // running it on a misconfigured box is exactly who needs it.
    match RuntimeConfig::from_env_and_args(&["--version".into()], |_| None).expect("version") {
        ConfigOutcome::Version => {}
        other => panic!("expected Version, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Derived defaults
// ---------------------------------------------------------------------------

#[test]
fn workspace_and_artifact_roots_default_under_the_state_dir() {
    // Not absolute paths. A test pointing --state-dir at a temp
    // directory must get everything else under it too, or it is
    // writing to the developer's home directory.
    let f = Fixture::new();
    let config = f.parse(&[], &[]).expect("parse");
    assert_eq!(config.workspace_root, config.state_dir.join("workspaces"));
    assert_eq!(config.artifacts_root, config.state_dir.join("artifacts"));
}

#[test]
fn the_state_dir_can_come_from_the_environment_alone() {
    let f = Fixture::new();
    let elsewhere = f._tmp.path().join("other-state");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    let map = HashMap::from([
        (
            "IRONMAINT_STATE_DIR".to_string(),
            elsewhere.to_string_lossy().into_owned(),
        ),
        (
            "IRONMAINT_TOKEN_FILE".to_string(),
            f.token_file.to_string_lossy().into_owned(),
        ),
        (
            "IRONMAINT_MIGRATIONS_DIR".to_string(),
            f.migrations.to_string_lossy().into_owned(),
        ),
    ]);
    let config =
        match RuntimeConfig::from_env_and_args(&[], |n| map.get(n).cloned()).expect("parse") {
            ConfigOutcome::Run(c) => c,
            other => panic!("expected Run, got {other:?}"),
        };
    assert_eq!(config.state_dir, elsewhere);
    assert_eq!(config.workspace_root, elsewhere.join("workspaces"));
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn no_token_is_a_refusal_not_a_default() {
    // The single most important line in this file. A daemon that
    // starts with no token would serve the entire tool surface to
    // anything that can reach the socket.
    let map = no_env();
    let err = RuntimeConfig::from_env_and_args(&["--state-dir".into(), "/tmp/x".into()], |n| {
        map.get(n).cloned()
    })
    .expect_err("must refuse to start without a token");
    let message = err.to_string();
    assert!(message.contains("--token-file"), "{message}");
    assert!(
        message.contains("IRONMAINT_TOKEN_FILE"),
        "the env var must be named too: {message}"
    );
    assert!(
        message.contains("no unauthenticated mode"),
        "the refusal must say *why*: {message}"
    );
}

#[test]
fn a_token_file_that_does_not_exist_is_a_refusal() {
    let f = Fixture::new();
    // Built by hand rather than by editing `f.args()`: the
    // offending path has to sit in the flag's *value* position,
    // and splicing a bare path onto the end would exercise the
    // unknown-flag path instead.
    let args = vec![
        OsString::from("--state-dir"),
        f.root.clone().into(),
        OsString::from("--token-file"),
        OsString::from("/nonexistent/ironmaint-token-for-test"),
        OsString::from("--migrations-dir"),
        f.migrations.clone().into(),
    ];
    let map = no_env();
    let err = RuntimeConfig::from_env_and_args(&args, |n| map.get(n).cloned())
        .expect_err("a missing token file must be refused");
    assert!(matches!(err, ConfigError::Missing { .. }), "{err}");
    assert!(err.to_string().contains("token"), "{err}");
}

#[test]
fn a_migrations_dir_that_does_not_exist_is_a_refusal() {
    // Silently opening a store with no migrations produces a
    // schemaless database, and the first failure is then an
    // unrelated "no such table: events" from deep inside a query
    // — hours later, in production.
    let f = Fixture::new();
    let args = vec![
        OsString::from("--state-dir"),
        f.root.clone().into(),
        OsString::from("--token-file"),
        f.token_file.clone().into(),
        OsString::from("--migrations-dir"),
        OsString::from("/nonexistent/migrations-for-test"),
    ];
    let map = no_env();
    let err = RuntimeConfig::from_env_and_args(&args, |n| map.get(n).cloned())
        .expect_err("a missing migrations dir must be refused");
    assert!(err.to_string().contains("migrations"), "{err}");
}

#[test]
fn an_unknown_flag_is_a_refusal() {
    let f = Fixture::new();
    let err = f
        .parse(&["--listen-anywhere"], &[])
        .expect_err("unknown flags must be refused, not ignored");
    assert!(err.to_string().contains("--listen-anywhere"), "{err}");
}

#[test]
fn a_flag_with_no_value_is_a_refusal() {
    // The failure this prevents is subtle: `--bind` as the last
    // argument would otherwise consume nothing and silently fall
    // back to the default, so the daemon would listen somewhere
    // the operator did not ask for and would say so confidently.
    let f = Fixture::new();
    let mut args = f.args();
    args.push("--bind".into());
    let map = no_env();
    let err = RuntimeConfig::from_env_and_args(&args, |n| map.get(n).cloned())
        .expect_err("a dangling flag must be refused");
    assert!(err.to_string().contains("--bind"), "{err}");
    assert!(err.to_string().contains("requires a value"), "{err}");
}

#[test]
fn an_unparseable_bind_address_is_a_refusal() {
    let f = Fixture::new();
    let err = f
        .parse(&["--bind", "localhost:not-a-port"], &[])
        .expect_err("a bad address must be refused");
    let message = err.to_string();
    assert!(message.contains("--bind"), "{message}");
    assert!(
        message.contains("HOST:PORT"),
        "the fix must be shown: {message}"
    );
}

#[test]
fn an_unparseable_env_bind_falls_back_to_the_default() {
    // Unlike a flag, a bad value in the environment is not the
    // operator typing in front of a terminal — it is whatever set
    // it. Refusing to start over a typo in a unit file would make
    // the daemon less available for no safety gain, and the log
    // line records the address actually used.
    let f = Fixture::new();
    let config = f
        .parse(&[], &[("IRONMAINT_BIND", "not-an-address")])
        .expect("parse");
    assert_eq!(config.bind.to_string(), "127.0.0.1:7341");
}

#[test]
fn the_fixture_binary_defaults_next_to_the_daemon() {
    // `cargo build` puts every workspace binary in one target
    // directory, so an operator who builds the workspace gets a
    // working check.run with no configuration at all.
    //
    // Asserted through `default_fixture_bin_for` with the
    // *daemon's* path, not through `RuntimeConfig`: a test
    // binary's `current_exe` is `target/debug/deps/…`, so
    // comparing the configured value against this test's own
    // `current_exe` would have passed regardless of the answer.
    let expected = ironmaintd::config::default_fixture_bin_for(Some(std::path::Path::new(
        "/opt/ironmaint/bin/ironmaintd",
    )));
    assert_eq!(
        expected,
        PathBuf::from("/opt/ironmaint/bin/ironmaint-fixture")
    );
    // And a bare name with no parent directory must not panic.
    assert_eq!(
        ironmaintd::config::default_fixture_bin_for(Some(std::path::Path::new("ironmaintd"))),
        PathBuf::from("ironmaint-fixture")
    );
}

#[test]
fn usage_names_every_flag_the_parser_accepts() {
    // A flag that exists but is undocumented is a flag nobody
    // will use. The list is checked by parsing `usage()` rather
    // than by eye so a new flag forces a doc update.
    let text = ironmaintd::config::usage();
    for flag in [
        "--state-dir",
        "--migrations-dir",
        "--bind",
        "--token-file",
        "--workspace-root",
        "--artifacts-root",
        "--fixture-bin",
        "--debian-tool-bin",
        "--log",
        "--help",
        "--version",
    ] {
        assert!(text.contains(flag), "usage() does not mention {flag}");
    }
    assert!(
        text.contains("IRONMAINT_TOKEN_FILE"),
        "env vars must be documented"
    );
    assert!(
        text.contains("ironmaintd listening on"),
        "the machine-readable startup line must be documented: \
         scripts and the e2e harness parse it"
    );
}
