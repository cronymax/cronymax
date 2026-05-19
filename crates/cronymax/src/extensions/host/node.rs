//! Node 26 host — spawn, monitor, restart.
//!
//! **Phase 2 implements this (`P2-T03`).** Today this is a typed stub so
//! callers can hold a `Handle` without depending on the concrete impl.

use std::path::PathBuf;

use crate::extensions::error::ExtensionResult;

/// Settings for spawning the Node host of one extension.
#[derive(Clone, Debug)]
pub struct SpawnConfig {
    pub ext_id: String,
    /// Path to the Node 26 binary cronymax ships.
    pub node_binary: PathBuf,
    /// Already-built argv tail; do NOT mutate inside the host module.
    pub node_flags: Vec<String>,
    /// Absolute path to `bundled/extension-host-bootstrap.js`.
    pub bootstrap_js: PathBuf,
    /// Per-extension storage dir.
    pub storage_dir: PathBuf,
}

/// Handle returned by [`Host::spawn`]. Phase 2 fills in real fields.
#[derive(Debug)]
pub struct Handle {
    pub ext_id: String,
}

#[derive(Debug, Default)]
pub struct Host;

impl Host {
    pub fn new() -> Self {
        Self
    }

    pub async fn spawn(&self, _cfg: SpawnConfig) -> ExtensionResult<Handle> {
        Err(crate::extensions::ExtensionError::HostSpawn(
            "Node host spawn is implemented in Phase 2 (P2-T03)".into(),
        ))
    }
}
