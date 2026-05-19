//! `ctx.storageUri` / `globalStorageUri` backing key-value store.
//!
//! Phase 3 (`P3-T04`) implements. Per-extension scopes; SQLite or JSON file
//! per extension (Phase 3 decision).

use crate::extensions::error::ExtensionResult;

pub async fn get(
    _ext_id: &str,
    _scope: StorageScope,
    _key: &str,
) -> ExtensionResult<Option<serde_json::Value>> {
    Ok(None)
}

pub async fn set(
    _ext_id: &str,
    _scope: StorageScope,
    _key: &str,
    _value: serde_json::Value,
) -> ExtensionResult<()> {
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageScope {
    /// `~/.cronymax/extensions/<id>/state.json` — workspace-bound storage.
    Workspace,
    /// `~/.cronymax/global-state/<id>.json` — cross-workspace storage.
    Global,
}
