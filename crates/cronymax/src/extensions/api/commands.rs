//! `commands.register` / `commands.execute` RPC handlers.
//!
//! Phase 2 (`P2-T06`) implements. Mirrors `cep-idl/v1/commands.ts`.
//! Cross-extension `execute` dispatch lands in Phase 4.

use std::collections::HashMap;

use crate::extensions::error::{ExtensionError, ExtensionResult};

#[derive(Debug, Default)]
pub struct CommandRegistry {
    /// `command_id` → owning extension id. Populated when Phase 2 wires
    /// `commands/register` to call `CommandRegistry::register`.
    #[allow(dead_code)]
    owners: HashMap<String, String>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, _ext_id: &str, _command_id: &str) -> ExtensionResult<()> {
        Err(ExtensionError::RpcMethodMissing(
            "commands/register is implemented in Phase 2 (P2-T06)".into(),
        ))
    }

    pub async fn execute(
        &self,
        _command_id: &str,
        _args: Vec<serde_json::Value>,
    ) -> ExtensionResult<serde_json::Value> {
        Err(ExtensionError::RpcMethodMissing(
            "commands/execute is implemented in Phase 2 (P2-T06)".into(),
        ))
    }
}
