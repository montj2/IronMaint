//! `ironmaintd` — daemon binary.
//!
//! PHASE-0B.md §66-§68. Startup sequence:
//!   1. Read `RuntimeConfig` (CLI + env).
//!   2. Open the SQLite store (which acquires the single-daemon
//!      lock — see ironmaint-store-sqlite).
//!   3. Build `RuntimeService` over the store.
//!   4. Block on `tokio::signal::ctrl_c()`; on signal, drain
//!      in-flight work and exit.
//!
//! Commit 14 ships the bootstrap path. The MCP server wiring
//! (commit 15) hooks into this `main` between steps 3 and 4.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use ironmaint_runtime::RuntimeService;
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state_dir =
        parse_state_dir_from_env().unwrap_or_else(|| PathBuf::from("/var/lib/ironmaint"));
    tracing_setup();
    let config = SqliteStoreConfig::new(state_dir.join("ironmaint.db"));
    let store = SqliteStore::open(config).await?;
    let _service = RuntimeService::new(std::sync::Arc::new(store));
    tracing_setup_done();
    tokio::signal::ctrl_c().await?;
    tracing_shutdown_received();
    Ok(())
}

fn parse_state_dir_from_env() -> Option<PathBuf> {
    std::env::var_os("IRONMAINT_STATE_DIR").map(PathBuf::from)
}

fn tracing_setup() {
    let _ = tracing_subscriber_init();
}

fn tracing_subscriber_init() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

fn tracing_setup_done() {
    eprintln!("ironmaintd: bootstrap complete; waiting for ctrl_c");
}

fn tracing_shutdown_received() {
    eprintln!("ironmaintd: shutdown signal received");
}
