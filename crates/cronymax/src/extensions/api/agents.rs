//! `cronymax.agents.provider` runtime registry.
//!
//! Phase 4 (`P4-T05`) implements — **the keystone L2 EP**. Mirrors
//! `cep-idl/v1/agents.ts`. The same registry is consumed by both the chat
//! panel and the flow runtime; that symmetry is a spec invariant (plan §2.4).

use std::collections::HashMap;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// Provider record stored after `agents.registerProvider("id", impl)`.
#[derive(Debug)]
pub struct ProviderEntry {
    pub provider_id: String,
    pub owning_ext: String,
    /// Phase 4 will define the cross-process handle shape.
    pub handle: ProviderHandle,
}

/// Opaque RPC handle to the JS-side implementation.
#[derive(Debug)]
pub struct ProviderHandle {
    pub _ext_id: String,
}

#[derive(Debug, Default)]
pub struct AgentProviderRegistry {
    /// `provider_id` → entry. Provider ids live in the publisher namespace.
    /// Phase 4 (`P4-T05`) populates this from `AgentProviderRegistry::register`.
    #[allow(dead_code)]
    entries: HashMap<String, ProviderEntry>,
}

impl AgentProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, _entry: ProviderEntry) -> ExtensionResult<()> {
        Err(ExtensionError::RpcMethodMissing(
            "agents.registerProvider is implemented in Phase 4 (P4-T05)".into(),
        ))
    }

    pub fn get(&self, provider_id: &str) -> Option<&ProviderEntry> {
        self.entries.get(provider_id)
    }

    pub fn list(&self) -> impl Iterator<Item = &ProviderEntry> {
        self.entries.values()
    }
}
