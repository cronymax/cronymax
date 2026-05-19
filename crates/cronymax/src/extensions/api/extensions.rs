//! `extensions.getExtension` / `extensions.all` RPC handlers.
//!
//! Phase 3 (`P3-T07`) implements. VS Code-style cross-extension hand-off via
//! `Extension.exports` — no schema, no semver, contract is between the two
//! extensions only. Mirrors `cep-idl/v1/extensions.ts`.

use crate::extensions::error::ExtensionResult;

#[derive(Clone, Debug)]
pub struct ExtensionView {
    pub id: String,
    pub is_active: bool,
    pub exports: serde_json::Value,
}

pub async fn get(_id: &str) -> ExtensionResult<Option<ExtensionView>> {
    Ok(None)
}

pub async fn list() -> ExtensionResult<Vec<ExtensionView>> {
    Ok(Vec::new())
}
