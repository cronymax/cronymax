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
//! Platform inserts ProviderEntry { provider_id, owning_ext, conn }
//!     ↓
//! Chat panel / flow runtime calls AgentProviderRegistry::get(...) and
//! uses the entry's `conn` to send `agents/session.create`, then
//! `agents/session.prompt`, and receives streamed events back via
//! "agents/event" notify
//! ```
//!
//! Wire layer (RPC method names) lives in [`crate::extensions::rpc::codec::agents_method`].

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::extensions::error::{ExtensionError, ExtensionResult};
use crate::extensions::rpc::Connection;

/// One registered agent provider. The `conn` is the live RPC channel to
/// the extension's Node host; the chat panel uses it to drive sessions.
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
    /// Live RPC connection to the owning extension. Cheap to clone.
    pub conn: Arc<Connection>,
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
    use crate::extensions::rpc::RpcServer;
    use tokio::io::duplex;

    async fn fake_conn() -> Arc<Connection> {
        // Two ends of a duplex talking to themselves; we never write
        // anything in these tests, just need a valid Connection handle.
        let (a, _b) = duplex(64);
        let (r, w) = tokio::io::split(a);
        let (conn, _task) = Connection::open(r, w, RpcServer::builder().build());
        conn
    }

    fn entry(provider_id: &str, ext_id: &str, conn: Arc<Connection>) -> ProviderEntry {
        ProviderEntry {
            provider_id: provider_id.into(),
            owning_ext: ext_id.into(),
            label: format!("Label for {provider_id}"),
            icon: None,
            description: None,
            supports_models: true,
            supports_modes: false,
            supports_mcp: false,
            conn,
        }
    }

    #[tokio::test]
    async fn register_then_lookup() {
        let reg = AgentProviderRegistry::new();
        let conn = fake_conn().await;
        reg.register(entry("alice.x.gpt", "alice.x", conn)).unwrap();
        let got = reg.get("alice.x.gpt").unwrap();
        assert_eq!(got.label, "Label for alice.x.gpt");
        assert_eq!(reg.len(), 1);
    }

    #[tokio::test]
    async fn cronymax_namespace_rejected() {
        let reg = AgentProviderRegistry::new();
        let conn = fake_conn().await;
        let err = reg
            .register(entry("cronymax.builtin", "alice.x", conn))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NamespaceReserved(_)));
    }

    #[tokio::test]
    async fn cross_extension_id_collision_rejected() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("shared", "alice.x", fake_conn().await))
            .unwrap();
        let err = reg
            .register(entry("shared", "bob.y", fake_conn().await))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
        // First entry still present.
        assert_eq!(reg.get("shared").unwrap().owning_ext, "alice.x");
    }

    #[tokio::test]
    async fn same_extension_can_reregister() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("p1", "alice.x", fake_conn().await))
            .unwrap();
        // Re-register with a new conn (simulating re-activate after crash).
        reg.register(entry("p1", "alice.x", fake_conn().await))
            .unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[tokio::test]
    async fn unregister_only_works_for_owner() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("p1", "alice.x", fake_conn().await))
            .unwrap();
        assert!(reg.unregister("bob.y", "p1").is_err());
        assert!(reg.unregister("alice.x", "p1").unwrap());
        assert!(reg.get("p1").is_none());
        // Second unregister: returns Ok(false) — not present.
        assert!(!reg.unregister("alice.x", "p1").unwrap());
    }

    #[tokio::test]
    async fn unregister_all_for_drops_only_that_extensions_providers() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("a1", "alice.x", fake_conn().await))
            .unwrap();
        reg.register(entry("a2", "alice.x", fake_conn().await))
            .unwrap();
        reg.register(entry("b1", "bob.y", fake_conn().await))
            .unwrap();
        let dropped = reg.unregister_all_for("alice.x");
        assert_eq!(dropped, 2);
        assert!(reg.get("b1").is_some());
    }

    #[tokio::test]
    async fn list_is_sorted_by_provider_id() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("zeta", "alice.x", fake_conn().await))
            .unwrap();
        reg.register(entry("alpha", "alice.x", fake_conn().await))
            .unwrap();
        reg.register(entry("mu", "alice.x", fake_conn().await))
            .unwrap();
        let ids: Vec<String> = reg.list().into_iter().map(|e| e.provider_id).collect();
        assert_eq!(ids, vec!["alpha", "mu", "zeta"]);
    }
}
