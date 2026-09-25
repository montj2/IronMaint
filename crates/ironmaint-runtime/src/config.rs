//! Runtime configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeConfig {
    pub state_dir: PathBuf,
    pub listen_addr: SocketAddr,
    pub log_level: String,
}

impl RuntimeConfig {
    #[must_use]
    pub fn minimal(state_dir: PathBuf) -> Self {
        // SAFETY: "127.0.0.1:0" is a syntactically valid
        // SocketAddr; the parse can only fail if the literal is
        // changed. We use unwrap_or with a loopback default to
        // honour the panic-policy lints.
        let listen_addr = "127.0.0.1:0"
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0)));
        Self {
            state_dir,
            listen_addr,
            log_level: "info".to_string(),
        }
    }
}
