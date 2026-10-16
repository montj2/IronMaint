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

use ironmaint_core::SourceImportPolicy;

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
    /// PHASE-2 PR 2.0-E: parsed `SourceImportPolicy` set for
    /// the `source.import` MCP tool. Each entry is built via
    /// `SourceImportPolicy::parse`; see `ironmaint-core`'s
    /// `source_import` module for the policy grammar.
    /// Empty by default — the tool rejects every URL out
    /// of the box.
    pub source_import_allowlist: Vec<SourceImportPolicy>,
    /// PHASE-1.1 PR 1.1.I (executor env boundary): path to
    /// the `bts-helper.py` subprocess. When set, the
    /// `debian.issue.bts_snapshot` `ToolDefinition` carries
    /// `BTS_HELPER=<this path>` as its per-tool env (see
    /// `ToolDefinition::env`); the helper script's path is
    /// therefore config-supplied, not picked up from the
    /// daemon's ambient env. When unset, the bts-snapshot
    /// tool's env is empty and the `bts-snapshot` subcommand
    /// falls back to its existing hermetic path (no
    /// `BTS_HELPER` in its now-closed env, so the subcommand
    /// returns the hard-coded fixture). Default: `None`.
    pub bts_helper: Option<PathBuf>,
    /// PHASE-2 PR 2.0-C (lock the build execution model,
    /// `doc/PHASE-2-C.md` §2): the directory under which
    /// `sbuild` creates its schroots on first use. The
    /// `containers/debian-tools` compose service mounts
    /// the named volume `ironmaint_sbuild-buildroots` at
    /// this path; the daemon's startup line records the
    /// resolved value. Operator-overridable via
    /// `--sbuild-buildroot-dir` (env-var fallback
    /// `IRONMAINT_SBUILD_BUILDROOT_DIR`). Default:
    /// `/var/lib/sbuild`.
    pub sbuild_buildroot_dir: PathBuf,
    /// PHASE-2 PR 2.0-C (`doc/PHASE-2-C.md` §5): whether
    /// `sbuild` runs under unprivileged user namespaces
    /// (the `sbuild --unshare` backend). Default: `true`
    /// (unprivileged). The operator can opt out with
    /// `--no-sbuild-unprivileged` (env-var fallback
    /// `IRONMAINT_SBUILD_UNPRIVILEGED=false`), in which
    /// case the schroot backend is used and the container
    /// must be run with `--privileged`. The default is
    /// the unprivileged mode; the opt-out is for runtimes
    /// whose user-namespace support is disabled.
    pub sbuild_unprivileged: bool,
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
        // PHASE-2 PR 2.0-E: repeatable CLI flag, one
        // `SourceImportPolicy` per occurrence, parsed via
        // `SourceImportPolicy::parse`. The env var is
        // comma-separated so an operator with several
        // policies does not have to set the flag N times
        // in a unit file. A parse failure is a startup
        // error, not a per-call error — the operator must
        // fix the config and restart.
        let mut source_import_allowlist: Vec<SourceImportPolicy> = Vec::new();
        // PHASE-1.1 PR 1.1.I: the bts-helper path is
        // config-supplied; the daemon passes it to the
        // bts-snapshot tool's per-tool env so the helper
        // script's path is no longer picked up from
        // ambient env. CLI flag takes precedence; the
        // `IRONMAINT_BTS_HELPER` env var is the fallback
        // (the same precedence as `--source-import-allowlist`
        // vs `IRONMAINT_SOURCE_IMPORT_ALLOWLIST`).
        let mut bts_helper: Option<PathBuf> = None;
        // PHASE-2 PR 2.0-C: the build-execution knobs
        // (buildroot dir + sandbox mode). Same precedence
        // as the other flags: flag beats env var beats
        // default. The unprivileged mode is the
        // explicit-default + opt-out shape: the flag
        // carries an `Option<bool>` (`Some(true)` or
        // `Some(false)`), the env var carries a string
        // the parser maps to bool, and the default is
        // `true` (the unprivileged mode is what the
        // compose service expects).
        let mut sbuild_buildroot_dir: Option<PathBuf> = None;
        let mut sbuild_unprivileged: Option<bool> = None;

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
                // PHASE-1.1 PR 1.1.G: repeatable; each
                // occurrence adds one URL prefix to the
                // allowlist. `take` consumes the next
                // argument and advances `index`. Each entry
                // is parsed via `SourceImportPolicy::parse`;
                // a parse failure is a startup error.
                "--source-import-allowlist" => {
                    let raw = take("--source-import-allowlist")?;
                    source_import_allowlist.push(SourceImportPolicy::parse(&raw).map_err(|e| {
                        ConfigError::InvalidValue {
                            what: "--source-import-allowlist",
                            value: raw.clone(),
                            why: e.to_string(),
                        }
                    })?);
                }
                "--bts-helper" => {
                    bts_helper = Some(PathBuf::from(take("--bts-helper")?));
                }
                // PHASE-2 PR 2.0-C: build-execution
                // knobs. `--sbuild-buildroot-dir` takes a
                // path; `--sbuild-unprivileged` and its
                // negator `--no-sbuild-unprivileged` carry
                // no value.
                "--sbuild-buildroot-dir" => {
                    sbuild_buildroot_dir = Some(PathBuf::from(take("--sbuild-buildroot-dir")?));
                }
                "--sbuild-unprivileged" => sbuild_unprivileged = Some(true),
                "--no-sbuild-unprivileged" => sbuild_unprivileged = Some(false),
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
        // PHASE-2 PR 2.0-E: the env var is comma-separated
        // (the flag is repeatable). Whitespace around entries
        // is trimmed so a config file with newlines survives
        // the parse. Each entry is parsed via
        // `SourceImportPolicy::parse`; a parse failure is a
        // startup error. The flag, if given, takes
        // precedence; the env var is only consulted when no
        // flag was set.
        if source_import_allowlist.is_empty()
            && let Some(raw) = get_env("IRONMAINT_SOURCE_IMPORT_ALLOWLIST")
        {
            for entry in raw.split(',') {
                let trimmed = entry.trim();
                if trimmed.is_empty() {
                    continue;
                }
                source_import_allowlist.push(SourceImportPolicy::parse(trimmed).map_err(|e| {
                    ConfigError::InvalidValue {
                        what: "IRONMAINT_SOURCE_IMPORT_ALLOWLIST",
                        value: trimmed.to_string(),
                        why: e.to_string(),
                    }
                })?);
            }
        }
        // PHASE-1.1 PR 1.1.I: the bts-helper path can also
        // come from the `IRONMAINT_BTS_HELPER` env var. The
        // flag, if given, takes precedence (same shape as
        // `--source-import-allowlist`).
        if bts_helper.is_none()
            && let Some(raw) = get_env("IRONMAINT_BTS_HELPER")
        {
            bts_helper = Some(PathBuf::from(raw));
        }
        // PHASE-2 PR 2.0-C: env-var fallbacks for the
        // build-execution knobs. The flag, if given, takes
        // precedence; the env var is the fallback; the
        // default is the compiled-in value. The
        // `IRONMAINT_SBUILD_UNPRIVILEGED` env var carries
        // `true` / `false` / `1` / `0` / `yes` / `no`
        // (case-insensitive); anything else is treated as
        // unset, so a typo does not silently flip the
        // sandbox mode.
        let sbuild_buildroot_dir = sbuild_buildroot_dir
            .or_else(|| get_env("IRONMAINT_SBUILD_BUILDROOT_DIR").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/var/lib/sbuild"));
        if sbuild_unprivileged.is_none()
            && let Some(raw) = get_env("IRONMAINT_SBUILD_UNPRIVILEGED")
        {
            sbuild_unprivileged =
                Some(
                    parse_boolish(&raw).ok_or_else(|| ConfigError::InvalidValue {
                        what: "IRONMAINT_SBUILD_UNPRIVILEGED",
                        value: raw.clone(),
                        why: "expected true/false, yes/no, 1/0 (case-insensitive)".to_string(),
                    })?,
                );
        }
        let sbuild_unprivileged = sbuild_unprivileged.unwrap_or(true);

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

        Ok(ConfigOutcome::Run(Box::new(RuntimeConfig {
            state_dir,
            migrations_dir,
            bind,
            token_file,
            workspace_root,
            artifacts_root,
            fixture_bin,
            debian_tool_bin,
            log_filter,
            source_import_allowlist,
            bts_helper,
            sbuild_buildroot_dir,
            sbuild_unprivileged,
        })))
    }
}

/// What the argument list asked the daemon to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOutcome {
    /// Start with this configuration.
    Run(Box<RuntimeConfig>),
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

/// Parse a "boolish" string (true/false, yes/no, 1/0,
/// case-insensitive). Returns `None` for any other
/// value, so an unparseable env var is treated as
/// unset rather than silently flipped. The shape is
/// the same as `clap`'s `BoolishValueParser`; we
/// hand-roll it because §86 does not sanction a CLI
/// parser dep, and the four-state parsing is small.
fn parse_boolish(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
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
    --source-import-allowlist PREFIX
                           URL prefix the `source.import` MCP
                           tool will accept (1.1.G). Repeatable.
                           [IRONMAINT_SOURCE_IMPORT_ALLOWLIST, none]
    --bts-helper PATH      Path to the `bts-helper.py`
                           subprocess (1.1.I). When set, the
                           bts-snapshot tool's env carries
                           `BTS_HELPER=<this path>`; the helper
                           path is config-supplied, not picked
                           up from the daemon's ambient env.
                           [IRONMAINT_BTS_HELPER, unset]
    --sbuild-buildroot-dir PATH
                           Directory under which `sbuild`
                           creates its schroots on first use
                           (2.0-C). The `debian-tools` compose
                           service mounts the named volume
                           `ironmaint_sbuild-buildroots` here.
                           [IRONMAINT_SBUILD_BUILDROOT_DIR, /var/lib/sbuild]
    --sbuild-unprivileged  Use unprivileged user namespaces
                           for `sbuild` (the `sbuild --unshare`
                           backend). The default; preferred.
                           [IRONMAINT_SBUILD_UNPRIVILEGED, true]
    --no-sbuild-unprivileged
                           Disable unprivileged user namespaces;
                           fall back to the schroot backend
                           (requires `--privileged` on the
                           container). For runtimes whose
                           user-namespace support is disabled.
                           [IRONMAINT_SBUILD_UNPRIVILEGED, true]
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
