//! `workspace.getConfiguration` backend.
//!
//! Phase 3 (`P3-T02`) implements. Mirrors `cep-idl/v1/workspace.ts`.

use crate::extensions::error::ExtensionResult;

pub async fn get(
    _ext_id: &str,
    _section: Option<&str>,
    _key: &str,
) -> ExtensionResult<serde_json::Value> {
    Ok(serde_json::Value::Null)
}

pub async fn update(
    _ext_id: &str,
    _section: Option<&str>,
    _key: &str,
    _value: serde_json::Value,
) -> ExtensionResult<()> {
    Ok(())
}
