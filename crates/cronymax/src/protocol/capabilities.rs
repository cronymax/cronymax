//! Capabilities surface — runtime-initiated, host-fulfilled privileged
//! operations.
//!
//! The runtime *requests* privileged work; the host *executes* it and
//! returns a typed response correlated by id. The host never decides
//! semantics — only whether and how to execute the requested operation
//! against local OS resources.
//!
//! Concrete capability families are sketched here so the dispatch
//! plumbing can be exercised end-to-end. Per-capability detail lives in
//! the `host-capability-adapter` spec and lands in tasks 6.x.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Runtime-issued capability invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "snake_case")]
pub enum CapabilityRequest {
    /// Run a sandboxed shell command and stream/aggregate output.
    Shell {
        /// Logical Space the command should be scoped to (workspace
        /// roots, environment, sandbox profile).
        space_id: String,
        argv: Vec<String>,
        cwd: Option<String>,
        env: Vec<(String, String)>,
        /// If set, runtime expects line-by-line streaming via separate
        /// trace events rather than a single aggregated response.
        stream: bool,
    },

    /// Inspect or mutate an active browser/page in the host.
    Browser {
        space_id: String,
        action: serde_json::Value,
    },

    /// Mediated filesystem access against a workspace root.
    Filesystem {
        space_id: String,
        op: serde_json::Value,
    },

    /// Read or write a host-managed secret.
    Secret {
        scope: String,
        op: SecretOp,
    },

    /// Surface a notification to the user (system tray, dock badge).
    Notify {
        title: String,
        body: String,
        level: NotifyLevel,
    },

    /// Ask the user for an interactive approval. The host is
    /// responsible for rendering UI; the response carries the user's
    /// decision.
    UserApproval {
        run_id: String,
        review_id: String,
        prompt: serde_json::Value,
    },
}

/// Read/write operations on host-managed secrets.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SecretOp {
    Get { key: String },
    Set { key: String, value: String },
    Delete { key: String },
}

/// Notification severity levels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    Info,
    Warning,
    Error,
}

/// Host reply to a capability invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CapabilityResponse {
    /// Successful execution. `payload` shape depends on the originating
    /// `CapabilityRequest` variant; concrete shapes land per-capability
    /// in tasks 6.x.
    Ok { payload: serde_json::Value },

    /// Typed failure. The runtime treats this as an authoritative
    /// outcome and decides the next state transition itself.
    Err { error: CapabilityError },
}

/// Typed capability failure.
#[derive(Clone, Debug, Serialize, Deserialize, Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CapabilityError {
    #[error("capability not supported by host: {capability}")]
    Unsupported { capability: String },

    #[error("permission denied: {reason}")]
    PermissionDenied { reason: String },

    #[error("invalid arguments: {message}")]
    InvalidArguments { message: String },

    #[error("execution failed: {message}")]
    ExecutionFailed { message: String },

    #[error("timed out after {millis}ms")]
    Timeout { millis: u64 },

    #[error("internal host error: {message}")]
    Internal { message: String },
}
