//! `auth.getSession` / `auth.removeSession` RPC handlers.
//!
//! Phase 3 (`P3-T06`) implements. Mirrors `cep-idl/v1/auth.ts`. Builds on
//! the platform's existing OAuth/PKCE/device-flow helpers (to be hoisted
//! out of provider-specific modules in Phase 3).

use crate::extensions::error::ExtensionResult;

#[derive(Clone, Debug)]
pub struct SessionRequest {
    pub ext_id: String,
    pub provider_id: String,
    pub scopes: Vec<String>,
    pub create_if_none: bool,
    pub force_new_session: bool,
}

pub async fn get_session(_req: SessionRequest) -> ExtensionResult<Option<serde_json::Value>> {
    Ok(None)
}

pub async fn remove_session(
    _ext_id: &str,
    _provider_id: &str,
    _session_id: &str,
) -> ExtensionResult<()> {
    Ok(())
}
