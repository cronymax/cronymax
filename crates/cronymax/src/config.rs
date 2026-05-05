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
}
