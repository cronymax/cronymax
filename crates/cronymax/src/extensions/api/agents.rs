//! `cronymax.agents.provider` runtime registry — the keystone L2 EP.
//!
//! Mirrors `cep-idl/v1/agents.ts`. The same registry is consumed by both
//! the chat panel and the flow runtime; that symmetry is a spec invariant
//! (plan §2.4). Layout:
//!
//! ```text
//! Extension declares contributes["cronymax.agents.provider"] in manifest
//!     ↓
//! On activate, ext calls `cronymax.agents.registerProvider(id, impl)`
//!     ↓ RPC notify "agents/registerProvider"
//! Platform inserts ProviderEntry { provider_id, owning_ext, …metadata… }
//!     ↓
//! Chat panel / flow runtime calls AgentProviderRegistry::get(...), then
//! drives the session via
//! `ExtensionRuntime::send_to_extension(&entry.owning_ext, "agents/session.create", …)`
//! and receives streamed events back via "agents/event" notify
//! ```
//!
//! `ProviderEntry` is **pure metadata** — it does not hold the host
//! connection. Connections live exclusively in
//! [`crate::extensions::runtime::ExtensionRuntime`]'s handle map, which
//! is the single source of truth for "where is this extension's RPC
//! channel". This avoids the chicken-and-egg between "build handlers"
//! and "spawn host" (handlers used to need to capture an `Arc<Connection>`
//! that didn't exist yet); now handlers just stash metadata and the
//! runtime supplies the conn at call time.
//!
//! Wire layer (RPC method names) lives in [`crate::extensions::rpc::codec::agents_method`].

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// One registered agent provider. Pure metadata — the RPC connection
/// is looked up via the runtime's handle map at call time (see
/// [`crate::extensions::runtime::ExtensionRuntime::send_to_extension`]).
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub provider_id: String,
    pub owning_ext: String,
    /// Human-readable label from manifest (`AgentProviderContribution.label`).
    pub label: String,
    /// Optional manifest icon path.
    pub icon: Option<String>,
    /// Optional manifest description.
    pub description: Option<String>,
    /// Capability flags from manifest.
    pub supports_models: bool,
    pub supports_modes: bool,
    pub supports_mcp: bool,
}

/// Thread-safe map of `provider_id → entry`. Cheap to share.
#[derive(Debug, Default, Clone)]
pub struct AgentProviderRegistry {
    inner: Arc<RwLock<HashMap<String, ProviderEntry>>>,
}

impl AgentProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider from an extension. Errors if `provider_id` is
    /// already taken by a different extension, or if the id is in the
    /// `cronymax.*` reserved namespace.
    pub fn register(&self, entry: ProviderEntry) -> ExtensionResult<()> {
        if entry.provider_id.starts_with("cronymax.") || entry.provider_id == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(format!(
                "agent provider id `{}` is reserved by the platform",
                entry.provider_id
            )));
        }
        let mut g = self.inner.write();
        if let Some(existing) = g.get(&entry.provider_id) {
            if existing.owning_ext != entry.owning_ext {
                return Err(ExtensionError::BadContribution {
                    point: "cronymax.agents.provider".into(),
                    ext_id: entry.owning_ext.clone(),
                    reason: format!(
                        "provider id `{}` is already owned by `{}`",
                        entry.provider_id, existing.owning_ext
                    ),
                });
            }
            // Same owner re-registering — overwrite (e.g. on re-activate).
        }
        g.insert(entry.provider_id.clone(), entry);
        Ok(())
    }

    /// Unregister a single provider. Returns whether it was present.
    /// Errors if a different extension claims to be removing it.
    pub fn unregister(&self, ext_id: &str, provider_id: &str) -> ExtensionResult<bool> {
        let mut g = self.inner.write();
        let Some(existing) = g.get(provider_id) else {
            return Ok(false);
        };
        if existing.owning_ext != ext_id {
            return Err(ExtensionError::BadContribution {
                point: "cronymax.agents.provider".into(),
                ext_id: ext_id.to_string(),
                reason: format!(
                    "provider `{provider_id}` is owned by `{}`, not `{ext_id}`",
                    existing.owning_ext
                ),
            });
        }
        g.remove(provider_id);
        Ok(true)
    }

    /// Drop every provider owned by `ext_id` (called on deactivate).
    pub fn unregister_all_for(&self, ext_id: &str) -> usize {
        let mut g = self.inner.write();
        let before = g.len();
        g.retain(|_, e| e.owning_ext != ext_id);
        before - g.len()
    }

    pub fn get(&self, provider_id: &str) -> Option<ProviderEntry> {
        self.inner.read().get(provider_id).cloned()
    }

    /// All providers known to the registry, sorted by id. Used by the chat
    /// panel to render a "provider picker" dropdown.
    pub fn list(&self) -> Vec<ProviderEntry> {
        let mut v: Vec<ProviderEntry> = self.inner.read().values().cloned().collect();
        v.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
        v
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider_id: &str, ext_id: &str) -> ProviderEntry {
        ProviderEntry {
            provider_id: provider_id.into(),
            owning_ext: ext_id.into(),
            label: format!("Label for {provider_id}"),
            icon: None,
            description: None,
            supports_models: true,
            supports_modes: false,
            supports_mcp: false,
        }
    }

    #[test]
    fn register_then_lookup() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("alice.x.gpt", "alice.x")).unwrap();
        let got = reg.get("alice.x.gpt").unwrap();
        assert_eq!(got.label, "Label for alice.x.gpt");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn cronymax_namespace_rejected() {
        let reg = AgentProviderRegistry::new();
        let err = reg
            .register(entry("cronymax.builtin", "alice.x"))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NamespaceReserved(_)));
    }

    #[test]
    fn cross_extension_id_collision_rejected() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("shared", "alice.x")).unwrap();
        let err = reg.register(entry("shared", "bob.y")).unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
        // First entry still present.
        assert_eq!(reg.get("shared").unwrap().owning_ext, "alice.x");
    }

    #[test]
    fn same_extension_can_reregister() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("p1", "alice.x")).unwrap();
        // Re-register (simulating re-activate after crash).
        reg.register(entry("p1", "alice.x")).unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn unregister_only_works_for_owner() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("p1", "alice.x")).unwrap();
        assert!(reg.unregister("bob.y", "p1").is_err());
        assert!(reg.unregister("alice.x", "p1").unwrap());
        assert!(reg.get("p1").is_none());
        // Second unregister: returns Ok(false) — not present.
        assert!(!reg.unregister("alice.x", "p1").unwrap());
    }

    #[test]
    fn unregister_all_for_drops_only_that_extensions_providers() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("a1", "alice.x")).unwrap();
        reg.register(entry("a2", "alice.x")).unwrap();
        reg.register(entry("b1", "bob.y")).unwrap();
        let dropped = reg.unregister_all_for("alice.x");
        assert_eq!(dropped, 2);
        assert!(reg.get("b1").is_some());
    }

    #[test]
    fn list_is_sorted_by_provider_id() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("zeta", "alice.x")).unwrap();
        reg.register(entry("alpha", "alice.x")).unwrap();
        reg.register(entry("mu", "alice.x")).unwrap();
        let ids: Vec<String> = reg.list().into_iter().map(|e| e.provider_id).collect();
        assert_eq!(ids, vec!["alpha", "mu", "zeta"]);
    }
}
