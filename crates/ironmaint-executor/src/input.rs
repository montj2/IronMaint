//! `ToolInputMode` — how a tool receives the `ExecutionRequest::input`
//! payload (PHASE-1.md §7).
//!
//! Phase 0B's `ProcessExecutor` always opened the child with
//! `Stdio::null()`, because every registered tool was a synthetic
//! fixture that did not consume input. Phase 1 introduces tools
//! that *do* — the first one is `synthetic.build.echo_input`,
//! whose job in the integration suite is to prove the wiring
//! works end-to-end; the production tools land in 1C.x
//! (`debian.inspect.source_preparation` et al.) and need the same
//! plumbing to receive the runtime-built candidate context.
//!
//! `None` is the default and the value every existing registration
//! site picks up, so the six synthetic keys keep their current
//! behaviour. `JsonStdin` is opt-in: the executor serialises
//! `ExecutionRequest::input` as JSON, writes it to the child's
//! stdin, then closes the pipe. The child is expected to read to
//! EOF.

use std::fmt;

use serde::{Deserialize, Serialize};

/// How a tool receives its input payload.
///
/// `Default::default()` is `None`, so a `ToolDefinitionRecord::new`
/// followed by no further builder call matches today's behaviour
/// (PHASE-0B.md §22). Adopters that add `with_input_mode(...)`
/// pay one extra spawned task per invocation — the writer that
/// feeds the child's stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ToolInputMode {
    /// No input is delivered to the child. The executor opens
    /// stdin as `Stdio::null()` and writes nothing.
    ///
    /// This is the right mode for tools whose `fixed_args` carry
    /// every input they need (the current synthetic suite), and
    /// for tools that read configuration from a file path passed
    /// via `env_overrides` rather than via stdin.
    #[default]
    None,
    /// The executor serialises `ExecutionRequest::input` as JSON,
    /// writes the bytes to the child's stdin, and closes the pipe.
    ///
    /// The wire format is single-line UTF-8 JSON followed by EOF.
    /// The child is expected to read to EOF; the executor does not
    /// insert a trailing newline. A child that calls `read_line`
    /// rather than `read_to_end` will lose every byte after the
    /// first newline — that is by design, because the input is a
    /// single JSON document, not a stream.
    JsonStdin,
}

impl fmt::Display for ToolInputMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("none"),
            Self::JsonStdin => f.write_str("json_stdin"),
        }
    }
}
