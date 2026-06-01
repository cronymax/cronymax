//! `events.on` / `events.emit` RPC handlers.
//!
//! Phase 3 (`P3-T01`) implements. Mirrors `cep-idl/v1/events.ts`.
//! Platform-emitted topics are owned by `super::super::events::PlatformEventBus`.

use crate::extensions::error::ExtensionResult;

pub async fn subscribe(_ext_id: &str, _topic: &str) -> ExtensionResult<()> {
    Ok(())
}

pub async fn publish(
    _ext_id: &str,
    _topic: &str,
    _payload: serde_json::Value,
) -> ExtensionResult<()> {
    Ok(())
}
