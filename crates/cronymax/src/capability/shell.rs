//! Sandboxed shell / PTY execution adapter (task 6.1).
//!
//! The agent loop calls `run_shell` as a named tool. The host
//! process executes the command in a sandbox (e.g. the PTY backend
//! in `app/terminal/`) and returns a structured [`ShellResult`].
//!
//! ## Safety contract
//!
//! This trait does **not** enforce the sandbox — that is the host's
//! responsibility. The runtime trusts the host's return value. The
//! approval gate (`NeedsApproval`) is the runtime-side safety net;
//! hosts that run commands without approval must not advertise
//! `NeedsApproval` from their dispatcher.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Input to a shell capability invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellRequest {
    /// Shell command string. Executed via `/bin/sh -c` by default
    /// unless the host's sandbox uses a different shell.
    pub command: String,
    /// Optional working directory inside the active workspace root.
    /// Relative paths are resolved against the space's workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Wall-clock timeout in seconds. `None` means the host default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u32>,
    /// Optional environment variable overrides. The host merges these
    /// on top of its default sandbox environment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
}

/// Exit status of a completed shell command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitStatus {
    /// Process exited with the given code.
    Code(i32),
    /// Process was killed by the host (e.g. timeout, memory limit).
    Killed,
    /// Process was signalled (macOS/Linux).
    Signal(i32),
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        matches!(self, ExitStatus::Code(0))
    }
}

/// Structured result returned by the host after running a command.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellResult {
    pub exit_status: ExitStatus,
    /// Combined stdout text (truncated to `max_output_bytes` if set).
    pub stdout: String,
    /// Combined stderr text (same truncation policy).
    pub stderr: String,
    /// How long the command ran, in milliseconds.
    pub elapsed_ms: u64,
}

/// Provider-facing interface for sandboxed shell execution. The
/// implementation lives in `crony/` and bridges to the terminal
/// backend in `app/terminal/`.
#[async_trait]
pub trait ShellCapability: Send + Sync + std::fmt::Debug {
    /// Run `request.command` in a sandboxed environment. Returns `Ok`
    /// even for non-zero exit codes — the caller decides what to do
    /// with a failed command. `Err` is reserved for infrastructure
    /// failures (executor crashed, timeout exceeded before any output,
    /// etc.).
    async fn run(&self, request: ShellRequest) -> anyhow::Result<ShellResult>;

    /// Maximum bytes of output the caller should request per tool call.
    /// Used by the dispatcher to populate the tool's JSON schema.
    fn max_output_bytes(&self) -> usize {
        32_768
    }
}
