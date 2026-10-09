//! `ironmaintd` — daemon binary.
//!
//! PHASE-0B.md §66-§68, startup sequence §67:
//!
//! ```text
//! load configuration
//!       ↓
//! acquire state-directory lock          (inside SqliteStore::open, §10)
//!       ↓
//! validate/create directories
//!       ↓
//! open SQLite
//!       ↓
//! run migrations
//!       ↓
//! verify artifact store
//!       ↓
//! recover interrupted operations        (deferred — Phase 1+)
//!       ↓
//! load adapter registry                 (deferred — Phase 1)
//!       ↓
//! load tool registry
//!       ↓
//! bind MCP
//!       ↓
//! READY
//! ```
//!
//! Three steps are marked deferred rather than silently skipped.
//! §67 says startup fails if any authoritative subsystem fails to
//! initialise, and a step that quietly does nothing is the
//! failure mode that rule exists to prevent. Each is named here,
//! in the log at `info`, and in
//! `doc/PHASE-0B-COMPLETION.md` §16, so that an operator reading
//! the startup output is not misled into thinking a capability
//! exists.
//!
//! Exit codes: `0` on a clean signal, `1` on a configuration or
//! startup failure, `2` on an unhandled runtime error. They are
//! distinct because a supervisor treats "refused to start" and
//! "crashed" differently, and collapsing them loses that.

use std::process::ExitCode;
use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_executor::{
    LimitsConfig, ProcessEnvironment, ProcessExecutor, ToolRegistry, factory_from_config,
};
use ironmaint_mcp::{IronMaintMcpServer, McpRuntime, TokenValidator, default_config, router};
use ironmaint_runtime::{AdapterRegistry, RuntimeService, SystemClock};
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use ironmaint_synthetic_tools::register_synthetic_tools;
use ironmaint_workspace::WorkspaceManager;
use tokio_util::sync::CancellationToken;

use ironmaintd::config::{ConfigOutcome, RuntimeConfig, usage};

/// Prefix of the single line printed on stdout once the socket is
/// accepting. Scripts and tests wait for this line to learn the
/// port, which is why `--bind 127.0.0.1:0` is usable at all: a
/// supervisor that can only read a log cannot otherwise find an
/// ephemeral port.
const LISTENING_PREFIX: &str = "ironmaintd listening on ";

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let config = match RuntimeConfig::from_env_and_args(&args, |name| std::env::var(name).ok()) {
        Ok(ConfigOutcome::Run(config)) => config,
        Ok(ConfigOutcome::Help) => {
            print!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Ok(ConfigOutcome::Version) => {
            println!("ironmaintd {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("ironmaintd: {e}");
            eprintln!("try `ironmaintd --help`");
            return ExitCode::FAILURE;
        }
    };

    init_tracing(&config.log_filter);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ironmaintd: cannot start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(run(*config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(StartupError::Config(e)) => {
            eprintln!("ironmaintd: {e}");
            ExitCode::FAILURE
        }
        Err(StartupError::Store(e)) => {
            eprintln!("ironmaintd: cannot open the store: {e}");
            ExitCode::FAILURE
        }
        Err(StartupError::Token(e)) => {
            eprintln!("ironmaintd: cannot read the MCP token: {e}");
            ExitCode::FAILURE
        }
        Err(StartupError::Serve(e)) => {
            eprintln!("ironmaintd: {e}");
            ExitCode::from(2)
        }
    }
}

/// A startup step that §67 requires to fail hard.
#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("{0}")]
    Config(#[from] ironmaintd::ConfigError),
    #[error("{0}")]
    Store(#[from] ironmaint_store::StoreError),
    #[error("{0}")]
    Token(#[from] ironmaint_mcp::AuthError),
    #[error("the MCP server stopped: {0}")]
    Serve(String),
}

/// Initialise `tracing` with a filter directive.
///
/// Writes to **stderr**, so that stdout carries exactly one line —
/// the `ironmaintd listening on …` contract — and nothing else.
/// A script that greps stdout for the port must not have to
/// filter ANSI-coloured log output out of it first, and an
/// operator redirecting stdout to a file should get the URL, not
/// a log.
///
/// `try_init` rather than `init`: a second subscriber would
/// panic, and there is no legitimate reason for this process to
/// install two, but a daemon that panics during startup because
/// of a library initialiser is a worse failure than one extra
/// log line from whatever got there first.
fn init_tracing(filter: &str) {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();
}

async fn run(config: RuntimeConfig) -> Result<(), StartupError> {
    // --- directories (§67 "validate/create directories") ---
    //
    // The store creates `state_dir` itself; the other two are
    // ours. A path that exists as a *file* is a configuration
    // mistake worth naming rather than a `create_dir_all` error
    // three frames deep.
    ensure_directory(&config.workspace_root, "workspace root")?;
    ensure_directory(&config.artifacts_root, "artifacts root")?;

    // --- open SQLite + acquire the lock (§10, §67) ---
    //
    // `SqliteStoreConfig::state_dir` is a *directory*: the
    // database file name and the lock file are the store's
    // business. Passing a file path here would create a directory
    // named after the file, which is how the previous version of
    // this function ended up with `<state>/ironmaint.db/`
    // containing the real database.
    let store = Arc::new(
        SqliteStore::open(
            SqliteStoreConfig::new(&config.state_dir).with_migrations_dir(&config.migrations_dir),
        )
        .await?,
    );

    // --- token (§51) ---
    //
    // Read once at startup and compared per request. Reading it
    // per request would let a token rotation take effect without
    // a restart, but would also make the daemon's auth depend on
    // filesystem state an attacker may be able to influence;
    // rotation is a restart, and the operator doc says so.
    let token = ironmaint_mcp::AuthToken::from_file(&config.token_file)?;
    let validator = TokenValidator::new(token);

    // --- artifact store (§67 "verify artifact store") ---
    //
    // Opened and kept alive for the process. `ProcessExecutor`
    // spills every tool run's complete stdout and stderr here, so
    // this is not a directory that happens to exist — it is where
    // the output a `check.run` bounded out of its record actually
    // goes. Before D-08's fix nothing was ever written to it while
    // startup logged that it was verified, which is the mismatch
    // that entry was raised for.
    let artifact_store = Arc::new(ArtifactStore::open(ArtifactRoot::new(
        &config.artifacts_root,
    )));

    // --- tool registry (§67) ---
    //
    // The six `synthetic.build.*` definitions, pointing at the
    // configured fixture binary. Registering these is what makes
    // `check.run` executable rather than refused; with an empty
    // registry every check call fails with "no tool registered",
    // which is a working transport that cannot do the one thing
    // it is for.
    let mut tools = ToolRegistry::new();
    let keys = register_synthetic_tools(&mut tools, &config.fixture_bin)
        .map_err(|e| StartupError::Serve(format!("tool registry: {e}")))?;
    // 1C.1: register the production Debian inspection tool.
    // The path is the configured `--debian-tool-bin`, which
    // defaults to `/nonexistent/ironmaint-debian-tool`; the
    // executor's pre-flight `check_launchable` rejects that
    // path, so a `check.run` against the key on a build that
    // does not include the binary returns
    // `InfrastructureFailed` rather than silently running a
    // different tool. The 1B.3 GREEN test asserts the
    // tool-not-found refusal still names the missing key
    // (so an agent can tell which gate is blocked).
    let source_preparation_key = ToolCapabilityKey::new("debian.inspect.source_preparation")
        .map_err(|e| StartupError::Serve(format!("debian.inspect.source_preparation key: {e}")))?;
    let source_preparation = ironmaint_executor::ToolDefinitionRecord::new(
        source_preparation_key.clone(),
        &config.debian_tool_bin,
        vec![std::ffi::OsString::from("source-preparation")],
        ironmaint_executor::ExecutionClass::Check,
        ironmaint_executor::ExecutionLimits {
            timeout: std::time::Duration::from_secs(30),
            stdout_max_bytes: 256 * 1024,
            stderr_max_bytes: 64 * 1024,
        },
    )
    .with_input_mode(ironmaint_executor::ToolInputMode::JsonStdin)
    .with_normalizer(std::sync::Arc::new(
        ironmaint_debian_tool::DebianSourcePreparationNormalizer::new(),
    ));
    tools
        .register(Box::new(source_preparation))
        .map_err(|e| StartupError::Serve(format!("debian.inspect.source_preparation: {e}")))?;
    tracing::info!(
        validate = %keys.validate,
        fail = %keys.fail,
        timeout = %keys.timeout,
        truncate = %keys.truncate,
        interrupt = %keys.interrupt,
        infra_fail = %keys.infra_fail,
        source_preparation = %source_preparation_key,
        fixture = %config.fixture_bin.display(),
        debian_tool_bin = %config.debian_tool_bin.display(),
        "tool registry loaded"
    );

    // --- executor ---
    //
    // `ProcessExecutor`, not `NullExecutor`. `NullExecutor`
    // reports every tool as an infrastructure failure, which is
    // the correct behaviour for a unit test and the *opposite* of
    // correct for a daemon: every check would come back
    // `InfrastructureFailed` and the agent would learn that
    // nothing about the package is knowable, which is a
    // believable-looking falsehood.
    // The registry is shared: the executor looks tools up by key
    // on every call, and the runtime consults the same set when
    // deciding whether a capability is registered at all. Two
    // copies would be two different answers to one question.
    let registry = Arc::new(tools);
    // §15's per-job retention caps. Wired here rather than left at
    // the executor's `permissive_guard` default: the caps are the
    // only thing standing between a job that runs twenty chatty
    // checks and an artifact store that fills the disk, and a
    // default that is never overridden is not a policy.
    let executor = Arc::new(
        ProcessExecutor::new(
            Arc::clone(&registry),
            Arc::clone(&artifact_store),
            ProcessEnvironment::new(),
        )
        .with_guard_factory(factory_from_config(LimitsConfig::default())),
    );

    // --- workspace (§66) ---
    let workspace = Arc::new(WorkspaceManager::new(
        &config.workspace_root,
        Arc::clone(&store),
    ));

    // --- adapter registry (§67) ---
    //
    // One process serves every distribution it has an adapter for.
    // The registry is keyed by each adapter's own declared family,
    // so this is where the two stubs enter the system; the runtime
    // never learns what "debian" means, only which adapter claims
    // it.
    //
    // What these stubs plan is real (§45 check planning, §48 policy
    // derivation) but not yet *runnable*: they name `debian.*` and
    // `fedora.*` tool keys, and the only tools registered in 0B are
    // the `synthetic.*` fixtures. A captured candidate therefore
    // activates, materialises its gates and derives its obligations
    // — and `check.run` on one of those gates reports the tool as
    // unknown, which is the honest answer. That visibility is the
    // point: the alternative was a job that silently never left
    // `EventDetected` (doc/DEBT.md D-03).
    let mut adapters = AdapterRegistry::empty();
    // 1B.3: the production Debian adapter replaces the Debian stub
    // in the daemon's adapter registration. The stub stays in the
    // workspace as a Phase 0 test fixture (the shared conformance
    // suite still walks it; §101's synthetic scenario uses it).
    // No production code path may instantiate `DebianStubAdapter`.
    adapters.register(Arc::new(debian::DebianAdapter::new()));
    adapters.register(Arc::new(fedora_stub::FedoraStubAdapter::new()));
    tracing::info!(
        families = ?adapters.families(),
        "adapter registry loaded"
    );

    // --- runtime service ---
    let service = Arc::new(
        RuntimeService::new(
            Arc::clone(&store),
            Arc::new(SystemClock),
            executor,
            registry,
        )
        .with_adapters(adapters)
        .with_artifact_store(Arc::clone(&artifact_store))
        .with_workspace(Arc::clone(&workspace)),
    );

    let runtime = McpRuntime::new(service).with_workspace(workspace);

    // --- bind MCP (§50) ---
    let shutdown = CancellationToken::new();
    let app = router(
        IronMaintMcpServer::new(runtime),
        validator,
        default_config(shutdown.clone()),
    );
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|e| StartupError::Serve(format!("cannot bind {}: {e}", config.bind)))?;
    let local = listener
        .local_addr()
        .map_err(|e| StartupError::Serve(format!("cannot read the bound address: {e}")))?;

    // Two lines, on two streams, for two audiences. The stdout
    // line is the machine-readable contract (`--bind :0` is only
    // usable if something can discover the port); the tracing line
    // is for the operator reading logs.
    println!("{LISTENING_PREFIX}http://{local}/mcp");
    tracing::info!(%local, "READY");
    tracing::info!(
        state_dir = %config.state_dir.display(),
        workspace_root = %config.workspace_root.display(),
        artifacts_root = %config.artifacts_root.display(),
        "configuration"
    );
    // §67's remaining step, stated rather than omitted. No
    // `PrivilegedOperation` is ever created today (Phase 0B §4.10,
    // §26, §99, and Phase 1's read-only-intake scope), so there is
    // nothing in flight to recover; the scan belongs to the
    // privileged service, which Phase 1 does not implement.
    tracing::info!(
        "recover interrupted operations: no privileged service in Phase 1, \
         so no operation can be in flight"
    );

    // --- serve until signalled (§68) ---
    //
    // Both SIGINT and SIGTERM. A daemon under a supervisor gets
    // SIGTERM; a daemon under a terminal gets SIGINT. Handling
    // only one means the other kills the process without the
    // graceful path, which is the difference between a drained
    // shutdown and a truncated SQLite write.
    let cancel = shutdown.clone();
    tokio::spawn(async move {
        if wait_for_signal().await {
            tracing::info!("signal received, draining");
            cancel.cancel();
        }
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
        .await
        .map_err(|e| StartupError::Serve(e.to_string()))?;

    tracing::info!("stopped");
    Ok(())
}

/// Wait for SIGINT or SIGTERM.
///
/// Returns `true` when a signal arrived. `false` means the
/// signal handlers could not be installed, in which case the
/// caller keeps serving: a daemon that cannot install a handler
/// should still run, and its liveness can be managed by
/// `SIGKILL` if necessary — silently exiting instead would be
/// worse.
async fn wait_for_signal() -> bool {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        // Each handler is installed independently. Falling back
        // to whichever one *did* install is better than exiting:
        // a daemon that cannot handle SIGTERM is still a daemon
        // that can handle SIGINT, and a supervisor that gets no
        // graceful shutdown will escalate to SIGKILL.
        //
        // `signal()` installs eagerly and returns `Result`;
        // `ctrl_c()` installs on first poll and reports failure
        // through its output.
        let mut term = match signal(SignalKind::terminate()) {
            Ok(term) => Some(term),
            Err(e) => {
                tracing::warn!("cannot install the SIGTERM handler ({e})");
                None
            }
        };
        let interrupt = tokio::signal::ctrl_c();
        match term.as_mut() {
            Some(term) => {
                tokio::select! {
                    result = interrupt => {
                        if let Err(e) = result {
                            tracing::error!("the SIGINT handler failed ({e}); waiting for SIGTERM");
                            term.recv().await;
                        } else {
                            tracing::info!("SIGINT");
                        }
                        true
                    }
                    _ = term.recv() => { tracing::info!("SIGTERM"); true }
                }
            }
            None => {
                match interrupt.await {
                    Ok(()) => tracing::info!("SIGINT"),
                    Err(e) => {
                        tracing::error!("no usable signal handler ({e})");
                        return false;
                    }
                }
                true
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        true
    }
}

/// Create a directory, or explain clearly that the path is
/// something else.
fn ensure_directory(path: &std::path::Path, what: &str) -> Result<(), StartupError> {
    if path.is_file() {
        return Err(StartupError::Serve(format!(
            "{what} {} exists and is a file, not a directory",
            path.display()
        )));
    }
    std::fs::create_dir_all(path)
        .map_err(|e| StartupError::Serve(format!("cannot create {what} {}: {e}", path.display())))
}
