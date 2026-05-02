//! Runtime configuration contract (task 1.4).
//!
//! `RuntimeConfig` is the canonical handshake structure that the host
//! (`crony`) hands to `cronymax::Runtime::start`. It captures everything
//! the runtime needs to come up without reaching back into the host:
//!
//! * Workspace roots — where user content lives.
//! * App-private storage — where the runtime keeps its own state.
//! * Logging configuration — log directory, level filter.
//! * Protocol version — for handshake / mismatch detection.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::protocol::ProtocolVersion;

/// Filesystem locations the runtime is allowed to read or write.
///
/// All paths are absolute. The host is responsible for creating any
/// missing directories before handing the config to the runtime.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoragePaths {
    /// Top-level workspace roots that the user has opened. The runtime
    /// must scope filesystem capabilities to these paths.
    pub workspace_roots: Vec<PathBuf>,

    /// App-private data directory owned by the runtime. Persistent run
    /// state, event journals, memory indexes, and permission grants
    /// live under here.
    pub app_data_dir: PathBuf,

    /// Cache directory the runtime may evict freely.
    pub cache_dir: PathBuf,
}

/// Logging configuration for the runtime process.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogConfig {
    /// Directory where rolling log files should be written.
    pub log_dir: PathBuf,

    /// `tracing`-style env filter directive (e.g. `"info,cronymax=debug"`).
    /// `None` lets the runtime fall back to `RUST_LOG` / its default.
    pub filter: Option<String>,
}

/// Sandbox policy the host communicates to the runtime per-workspace.
///
/// This is derived from the active space's `ProfileRecord` and the
/// workspace root path. When `None` is present in `RuntimeConfig`, the
/// runtime falls back to a permissive default policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Absolute path to the workspace root for this space.
    pub workspace_root: PathBuf,

    /// Whether outbound network access is allowed for this space.
    pub allow_network: bool,

    /// Additional absolute paths the runtime may read (beyond `workspace_root`).
    #[serde(default)]
    pub extra_read_paths: Vec<PathBuf>,

    /// Additional absolute paths the runtime may write (beyond `workspace_root`).
    #[serde(default)]
    pub extra_write_paths: Vec<PathBuf>,

    /// Absolute paths that must always be denied, regardless of other rules.
    #[serde(default)]
    pub extra_deny_paths: Vec<PathBuf>,
}

/// Configuration the host hands to the runtime at startup.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Filesystem layout the runtime is allowed to use.
    pub storage: StoragePaths,

    /// Logging configuration.
    pub logging: LogConfig,

    /// Protocol version the host expects to speak. The runtime fails
    /// fast if this is incompatible with `protocol::PROTOCOL_VERSION`.
    pub host_protocol: ProtocolVersion,

    /// Sandbox policy for the active workspace. `None` means use a
    /// permissive default (allow all network, workspace root only).
    #[serde(default)]
    pub sandbox: Option<SandboxConfig>,
}
