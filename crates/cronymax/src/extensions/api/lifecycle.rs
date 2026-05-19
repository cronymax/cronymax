//! `extension/activate` and `extension/deactivate` RPC handlers.
//!
//! Phase 2 (`P2-T06`) implements. Mirrors `cep-idl/v1/lifecycle.ts`.

use crate::extensions::error::ExtensionResult;

/// Args sent by `bootstrap.js` when it's ready to run user code.
#[derive(Debug)]
pub struct ActivateArgs {
    pub ext_id: String,
}

pub async fn activate(_args: ActivateArgs) -> ExtensionResult<()> {
    Ok(())
}

pub async fn deactivate(_ext_id: &str) -> ExtensionResult<()> {
    Ok(())
}
