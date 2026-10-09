//! `RuntimeConfig` — everything the daemon needs before it can bind.
//!
//! PHASE-0B.md §66 calls for an `IronMaintConfig` with a TOML
//! file and environment overrides. The file form is not
//! implemented, and the deviation is deliberate: §86's list of
//! new dependencies does not include a TOML parser, and the
//! eight values below are fully covered by flags and variables.
//! The precedence — flag beats variable beats default — is the
//! part operators depend on, so that is what is implemented and
//! tested. A TOML file, when it arrives, slots in beneath both.
//!
//! This lives in a library target rather than in `main.rs` so
//! that [`RuntimeConfig::from_env_and_args`] is reachable from
//! `tests/`. Config parsing is exactly the code where a silent
//! mistake produces a daemon that runs against the wrong
//! directory, and an untestable parser is how that happens.

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;

/// Default MCP listen address (PHASE-0B.md §50: `127.0.0.1`).
///
/// The port is 7341 and is overridable; the *host* is not,
/// because §50 pins loopback and a daemon that binds a routable
/// address with a bearer token is a different product. An
/// operator who needs that must say so explicitly with `--bind`.
pub const DEFAULT_BIND: &str = "127.0.0.1:7341";

/// Why the daemon could not be configured.
///
/// Every variant is fatal. A daemon that starts with a
/// half-understood configuration is worse than one that does not
/// start: it binds a socket, accepts agent traffic, and writes
/// state somewhere the operator did not choose.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// An unrecognised flag, or a flag missing its value.
    #[error("unknown or malformed argument: {0}")]
    Argument(String),

    /// A value that is not usable as given, e.g. an unparseable
    /// socket address.
    ///
    /// `why` is owned rather than `&'static str` because the
    /// useful form of the reason is the parser's own message, and
    /// interpolating a runtime error into a `&'static str` means
    /// either `Box::leak` (a leak on every bad flag) or dropping
    /// the detail.
    #[error("invalid value for {what}: {value} ({why})")]
    InvalidValue {
        /// The setting the value was given for.
        what: &'static str,
        /// What the operator typed.
        value: String,
        /// Why it could not be used.
        why: String,
    },

    /// A required setting was supplied but does not name
    /// something that exists.
    #[error("{what} does not exist: {value}")]
    Missing {
        /// The setting.
        what: &'static str,
        /// The path the operator gave.
        value: PathBuf,
    },
}

/// Fully-resolved daemon configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// Directory holding the SQLite database and the single-daemon
    /// lock. Created if absent. (Spec §66's `[database] path` is
    /// a filename inside this directory; the store names it, not
    /// the caller.)
    pub state_dir: PathBuf,
    /// Directory holding `NNNN_*.sql` migrations. Required —
    /// a store opened without it has no schema, and every query
    /// would fail with a confusing "no such table" much later.
    pub migrations_dir: PathBuf,
    /// Address the MCP transport binds.
    pub bind: SocketAddr,
    /// File whose contents are the bearer token. Required; there
    /// is no default and no unauthenticated mode.
    pub token_file: PathBuf,
    /// Root under which per-job working trees are created.
    pub workspace_root: PathBuf,
    /// Root under which tool output artifacts are written.
    pub artifacts_root: PathBuf,
    /// The `ironmaint-fixture` binary the synthetic tool
    /// definitions point at.
    pub fixture_bin: PathBuf,
    /// The `ironmaint-debian-tool` binary that backs the
    /// `debian.inspect.source_preparation` real check
    /// (PHASE-1.md §13, §16, 1C.1). The registration is
    /// unconditional — the tool is part of the production
    /// tool surface from 1C.1 onward — but the binary path
    /// defaults to `/nonexistent/ironmaint-debian-tool` so
    /// a pre-1C.1 build cannot accidentally run a binary
    /// it does not have. A `check.run` against the key on
    /// a build without the binary returns
    /// `InfrastructureFailed` (the executor's pre-flight
    /// `check_launchable` rejects the path).
    pub debian_tool_bin: PathBuf,
    /// `tracing` filter directive (e.g. `info`, `ironmaintd=debug`).
    pub log_filter: String,
}

impl RuntimeConfig {
    /// Resolve configuration from an argument list and the
    /// process environment.
    ///
    /// `args` excludes `argv[0]`. Both are taken as parameters
    /// rather than read from the process so this is testable and
    /// so a `--help` request can be answered without touching
    /// global state.
    ///
    /// Returns [`ConfigOutcome::Help`] or
    /// [`ConfigOutcome::Version`] for the two flags that print
    /// and exit rather than configuring anything.
    pub fn from_env_and_args(
        args: &[OsString],
        get_env: impl Fn(&str) -> Option<String>,
    ) -> Result<ConfigOutcome, ConfigError> {
        let mut state_dir: Option<PathBuf> = None;
        let mut migrations_dir: Option<PathBuf> = None;
        let mut bind: Option<SocketAddr> = None;
        let mut token_file: Option<PathBuf> = None;
        let mut workspace_root: Option<PathBuf> = None;
        let mut artifacts_root: Option<PathBuf> = None;
        let mut fixture_bin: Option<PathBuf> = None;
        let mut debian_tool_bin: Option<PathBuf> = None;
        let mut log_filter: Option<String> = None;

        // Flags whose value may be given as `--flag=value` or
        // `--flag value`. Hand-rolled because §86 does not
        // sanction a CLI-parsing dependency, and because a
        // daemon with eight flags does not justify one.
        let mut index = 0;
        while index < args.len() {
            let arg = args[index].to_string_lossy().into_owned();
            index += 1;

            // Split `--flag=value` before matching so the
            // single-dash and `--flag value` forms share one
            // table below.
            let (flag, inline) = match arg.split_once('=') {
                Some((flag, value)) if flag.starts_with('-') => {
                    (flag.to_string(), Some(value.to_string()))
                }
                _ => (arg.clone(), None),
            };

            let mut take = |name: &'static str| -> Result<String, ConfigError> {
                if let Some(value) = inline.clone() {
                    return Ok(value);
                }
                let value = args.get(index).map(|v| v.to_string_lossy().into_owned());
                index += 1;
                value.ok_or_else(|| ConfigError::Argument(format!("{name} requires a value")))
            };

            match flag.as_str() {
                "--help" | "-h" => return Ok(ConfigOutcome::Help),
                "--version" | "-V" => return Ok(ConfigOutcome::Version),
                "--state-dir" => state_dir = Some(PathBuf::from(take("--state-dir")?)),
                "--migrations-dir" => {
                    migrations_dir = Some(PathBuf::from(take("--migrations-dir")?));
                }
                "--bind" => {
                    let raw = take("--bind")?;
                    bind = Some(raw.parse().map_err(|e: std::net::AddrParseError| {
                        ConfigError::InvalidValue {
                            what: "--bind",
                            value: raw.clone(),
                            why: format!("expected HOST:PORT, e.g. 127.0.0.1:7341: {e}"),
                        }
                    })?);
                }
                "--token-file" => token_file = Some(PathBuf::from(take("--token-file")?)),
                "--workspace-root" => {
                    workspace_root = Some(PathBuf::from(take("--workspace-root")?));
                }
                "--artifacts-root" => {
                    artifacts_root = Some(PathBuf::from(take("--artifacts-root")?));
                }
                "--fixture-bin" => fixture_bin = Some(PathBuf::from(take("--fixture-bin")?)),
                "--debian-tool-bin" => {
                    debian_tool_bin = Some(PathBuf::from(take("--debian-tool-bin")?));
                }
                "--log" => log_filter = Some(take("--log")?),
                other => {
                    return Err(ConfigError::Argument(format!("unknown flag `{other}`")));
                }
            }
        }

        // Defaults are derived from `state_dir` rather than being
        // absolute, so a test that points `--state-dir` at a temp
        // directory gets everything else under it too and touches
        // nothing on the developer's machine.
        let state_dir = state_dir
            .or_else(|| get_env("IRONMAINT_STATE_DIR").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(".ironmaint"));
        let migrations_dir = migrations_dir
            .or_else(|| get_env("IRONMAINT_MIGRATIONS_DIR").map(PathBuf::from))
            .unwrap_or_else(default_migrations_dir);
        let bind = match bind
            .or_else(|| get_env("IRONMAINT_BIND").and_then(|raw| raw.parse().ok()))
            .or_else(|| DEFAULT_BIND.parse().ok())
        {
            Some(addr) => addr,
            None => {
                return Err(ConfigError::InvalidValue {
                    what: "--bind",
                    value: DEFAULT_BIND.to_string(),
                    why: "the compiled-in default is unparseable".to_string(),
                });
            }
        };
        let token_file = token_file
            .or_else(|| get_env("IRONMAINT_TOKEN_FILE").map(PathBuf::from))
            .ok_or_else(|| {
                ConfigError::Argument(
                    "no MCP token: pass --token-file PATH or set IRONMAINT_TOKEN_FILE. \
                     There is no default and no unauthenticated mode."
                        .to_string(),
                )
            })?;
        let workspace_root = workspace_root
            .or_else(|| get_env("IRONMAINT_WORKSPACE_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| state_dir.join("workspaces"));
        let artifacts_root = artifacts_root
            .or_else(|| get_env("IRONMAINT_ARTIFACTS_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| state_dir.join("artifacts"));
        let fixture_bin = fixture_bin
            .or_else(|| get_env("IRONMAINT_FIXTURE_BIN").map(PathBuf::from))
            .unwrap_or_else(|| default_fixture_bin_for(None));
        // The default `/nonexistent/...` is deliberate: 1C.1 makes
        // the `debian.inspect.source_preparation` registration part
        // of the production tool surface, but a build without the
        // binary must not silently run a different one. The
        // executor's pre-flight `check_launchable` rejects the
        // missing path and the tool reports
        // `InfrastructureFailed` — the same refusal the agent
        // would see if the binary were ever deleted in production.
        let debian_tool_bin = debian_tool_bin
            .or_else(|| get_env("IRONMAINT_DEBIAN_TOOL_BIN").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/nonexistent/ironmaint-debian-tool"));
        let log_filter = log_filter
            .or_else(|| get_env("IRONMAINT_LOG"))
            .unwrap_or_else(|| "info".to_string());

        // Existence is checked here rather than at the point of
        // use, so the operator gets every missing path in one
        // run instead of discovering them one restart at a time.
        if !token_file.is_file() {
            return Err(ConfigError::Missing {
                what: "MCP token file",
                value: token_file,
            });
        }
        if !migrations_dir.is_dir() {
            return Err(ConfigError::Missing {
                what: "migrations directory",
                value: migrations_dir,
            });
        }

        Ok(ConfigOutcome::Run(RuntimeConfig {
            state_dir,
            migrations_dir,
            bind,
            token_file,
            workspace_root,
            artifacts_root,
            fixture_bin,
            debian_tool_bin,
            log_filter,
        }))
    }
}

/// What the argument list asked the daemon to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOutcome {
    /// Start with this configuration.
    Run(RuntimeConfig),
    /// Print usage and exit 0.
    Help,
    /// Print the version and exit 0.
    Version,
}

/// Default migrations directory: the workspace's `migrations/`,
/// resolved at compile time from this crate's manifest.
///
/// This is a *development* default. A packaged install must pass
/// `--migrations-dir` (or set `IRONMAINT_MIGRATIONS_DIR`),
/// because a binary relocated to `/usr/bin` carries a build
/// machine's path inside it. The directory is checked for
/// existence at startup precisely so that mistake surfaces
/// immediately rather than as "no such table: events" on the
/// first agent call.
fn default_migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// Default fixture binary: a sibling of this executable.
///
/// `cargo build` puts every workspace binary in one target
/// directory, so "next to `ironmaintd`" finds
/// `ironmaint-fixture` without the operator naming it. It is
/// still only a default: a real deployment has no fixture
/// binary at all, and Phase 1 replaces these tools with genuine
/// adapter plans.
///
/// `exe` is the daemon's own executable path, or `None` to ask
/// the process for it. The parameter exists so the rule is
/// testable: a test binary's `current_exe` is
/// `target/debug/deps/…`, not `target/debug/`, so a test that
/// compared this function's result against its *own*
/// `current_exe` would pass no matter what the function returned.
#[must_use]
pub fn default_fixture_bin_for(exe: Option<&std::path::Path>) -> PathBuf {
    let owned;
    let exe = match exe {
        Some(path) => path,
        None => {
            owned = std::env::current_exe().ok();
            owned
                .as_deref()
                .unwrap_or(std::path::Path::new("ironmaintd"))
        }
    };
    exe.parent().map_or_else(
        || PathBuf::from("ironmaint-fixture"),
        |dir| dir.join("ironmaint-fixture"),
    )
}

/// Operator-facing usage text, printed by `--help`.
#[must_use]
pub fn usage() -> String {
    format!(
        "ironmaintd {version} — IronMaint package-maintenance control plane

USAGE:
    ironmaintd [OPTIONS]

Every option may also be given as an environment variable; the
flag wins. Options with no default are required.

OPTIONS:
    --state-dir PATH       State directory: SQLite database and the
                           single-daemon lock.  [IRONMAINT_STATE_DIR, ./.ironmaint]
    --migrations-dir PATH  Directory of NNNN_*.sql migrations.
                           [IRONMAINT_MIGRATIONS_DIR, <workspace>/migrations]
    --bind HOST:PORT       MCP listen address.
                           [IRONMAINT_BIND, {default_bind}]
    --token-file PATH      File holding the MCP bearer token. Required.
                           [IRONMAINT_TOKEN_FILE]
    --workspace-root PATH  Parent of per-job working trees.
                           [IRONMAINT_WORKSPACE_ROOT, <state-dir>/workspaces]
    --artifacts-root PATH  Parent of tool-output artifacts.
                           [IRONMAINT_ARTIFACTS_ROOT, <state-dir>/artifacts]
    --fixture-bin PATH     The synthetic build tool binary.
                           [IRONMAINT_FIXTURE_BIN, <exe-dir>/ironmaint-fixture]
    --debian-tool-bin PATH The real Debian inspection tool binary
                           (1C.1). The daemon registers
                           `debian.inspect.source_preparation` against
                           this path; a missing binary reports
                           `InfrastructureFailed`.
                           [IRONMAINT_DEBIAN_TOOL_BIN, /nonexistent/ironmaint-debian-tool]
    --log FILTER           tracing filter directive.
                           [IRONMAINT_LOG, info]
    -h, --help             Print this help.
    -V, --version          Print the version.

The daemon prints one line to stdout when it is listening:

    ironmaintd listening on http://127.0.0.1:7341/mcp

and exits 0 on SIGINT or SIGTERM.",
        version = env!("CARGO_PKG_VERSION"),
        default_bind = DEFAULT_BIND,
    )
}
