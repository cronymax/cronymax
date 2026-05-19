//! `secrets.get/set/delete` RPC handlers — OS keychain backend.
//!
//! Phase 3 (`P3-T03`) implements. Mirrors `cep-idl/v1/secrets.ts`. Backends:
//!
//! * macOS  → `security-framework` (already in workspace deps)
//! * Linux  → `secret-service` (added in Phase 3)
//! * Win    → DPAPI / Credential Manager (Phase 3)

use crate::extensions::error::ExtensionResult;

pub async fn get(_ext_id: &str, _namespace: &str, _key: &str) -> ExtensionResult<Option<String>> {
    Ok(None)
}

pub async fn set(_ext_id: &str, _namespace: &str, _key: &str, _value: &str) -> ExtensionResult<()> {
    Ok(())
}

pub async fn delete(_ext_id: &str, _namespace: &str, _key: &str) -> ExtensionResult<()> {
    Ok(())
}
