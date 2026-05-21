//! `cronymax.content.renderer` runtime registry.
//!
//! Mirrors `cep-idl/v1/renderers.ts`. The chat panel (and any other surface
//! that displays content blocks) looks up renderers by MIME type and asks
//! the owning extension to render an instance via
//! [`crate::extensions::runtime::ExtensionRuntime::send_to_extension`].
//!
//! Layout follows the same shape as [`super::agents::AgentProviderRegistry`]:
//! one shared `Arc<RwLock<...>>` and per-extension ownership accounting.
//! Entries are pure metadata; RPC connections live in the runtime's
//! handle map.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// One registered content renderer. Keyed in the registry by `renderer_id`.
#[derive(Debug, Clone)]
pub struct RendererEntry {
    pub renderer_id: String,
    pub owning_ext: String,
    /// MIME types this renderer claims, from the manifest.
    pub mime_types: Vec<String>,
    /// Entry path (relative to the extension dir) for the iframe.
    pub entry: String,
}

/// Thread-safe map keyed by `renderer_id`. Cheap to share.
#[derive(Debug, Default, Clone)]
pub struct ContentRendererRegistry {
    inner: Arc<RwLock<HashMap<String, RendererEntry>>>,
}

impl ContentRendererRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim a renderer id for `ext_id`. Re-registering the same
    /// `renderer_id` from the same `owning_ext` is allowed (re-activate
    /// idempotency). A different extension claiming the same id errors.
    pub fn register(&self, entry: RendererEntry) -> ExtensionResult<()> {
        if entry.renderer_id.starts_with("cronymax.") || entry.renderer_id == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(format!(
                "renderer id `{}` is reserved by the platform",
                entry.renderer_id
            )));
        }
        let mut g = self.inner.write();
        if let Some(existing) = g.get(&entry.renderer_id) {
            if existing.owning_ext != entry.owning_ext {
                return Err(ExtensionError::BadContribution {
                    point: "cronymax.content.renderer".into(),
                    ext_id: entry.owning_ext.clone(),
                    reason: format!(
                        "renderer id `{}` is already owned by `{}`",
                        entry.renderer_id, existing.owning_ext
                    ),
                });
            }
        }
        g.insert(entry.renderer_id.clone(), entry);
        Ok(())
    }

    pub fn unregister(&self, ext_id: &str, renderer_id: &str) -> ExtensionResult<bool> {
        let mut g = self.inner.write();
        let Some(existing) = g.get(renderer_id) else {
            return Ok(false);
        };
        if existing.owning_ext != ext_id {
            return Err(ExtensionError::BadContribution {
                point: "cronymax.content.renderer".into(),
                ext_id: ext_id.to_string(),
                reason: format!(
                    "renderer `{renderer_id}` is owned by `{}`, not `{ext_id}`",
                    existing.owning_ext
                ),
            });
        }
        g.remove(renderer_id);
        Ok(true)
    }

    pub fn unregister_all_for(&self, ext_id: &str) -> usize {
        let mut g = self.inner.write();
        let before = g.len();
        g.retain(|_, e| e.owning_ext != ext_id);
        before - g.len()
    }

    pub fn get(&self, renderer_id: &str) -> Option<RendererEntry> {
        self.inner.read().get(renderer_id).cloned()
    }

    /// First renderer that advertises this MIME type, if any. When more
    /// than one extension claims the same MIME the answer is sorted by
    /// `renderer_id` for deterministic selection.
    pub fn first_for_mime(&self, mime: &str) -> Option<RendererEntry> {
        let g = self.inner.read();
        let mut candidates: Vec<&RendererEntry> = g
            .values()
            .filter(|e| e.mime_types.iter().any(|m| m == mime))
            .collect();
        candidates.sort_by(|a, b| a.renderer_id.cmp(&b.renderer_id));
        candidates.first().map(|e| (*e).clone())
    }

    /// All renderers, sorted by `renderer_id`.
    pub fn list(&self) -> Vec<RendererEntry> {
        let mut v: Vec<RendererEntry> = self.inner.read().values().cloned().collect();
        v.sort_by(|a, b| a.renderer_id.cmp(&b.renderer_id));
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

    fn entry(renderer_id: &str, ext_id: &str, mimes: &[&str]) -> RendererEntry {
        RendererEntry {
            renderer_id: renderer_id.into(),
            owning_ext: ext_id.into(),
            mime_types: mimes.iter().map(|s| (*s).into()).collect(),
            entry: "./r.html".into(),
        }
    }

    #[test]
    fn register_then_lookup() {
        let r = ContentRendererRegistry::new();
        r.register(entry("mermaid", "acme.mermaid", &["text/x-mermaid"]))
            .unwrap();
        assert_eq!(r.get("mermaid").unwrap().owning_ext, "acme.mermaid");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn cronymax_namespace_rejected() {
        let r = ContentRendererRegistry::new();
        let err = r
            .register(entry("cronymax.builtin", "alice.x", &["text/plain"]))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NamespaceReserved(_)));
    }

    #[test]
    fn cross_extension_id_collision_rejected() {
        let r = ContentRendererRegistry::new();
        r.register(entry("shared", "alice.x", &["text/x-foo"]))
            .unwrap();
        let err = r
            .register(entry("shared", "bob.y", &["text/x-foo"]))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
        assert_eq!(r.get("shared").unwrap().owning_ext, "alice.x");
    }

    #[test]
    fn first_for_mime_returns_renderer() {
        let r = ContentRendererRegistry::new();
        r.register(entry("mermaid", "acme.mermaid", &["text/x-mermaid"]))
            .unwrap();
        let m = r.first_for_mime("text/x-mermaid").unwrap();
        assert_eq!(m.renderer_id, "mermaid");
        assert!(r.first_for_mime("nope/none").is_none());
    }

    #[test]
    fn first_for_mime_is_deterministic_on_collision() {
        let r = ContentRendererRegistry::new();
        // Two renderers both claim text/x-foo; sort wins.
        r.register(entry("zeta", "alice.x", &["text/x-foo"]))
            .unwrap();
        r.register(entry("alpha", "alice.x", &["text/x-foo"]))
            .unwrap();
        assert_eq!(r.first_for_mime("text/x-foo").unwrap().renderer_id, "alpha");
    }

    #[test]
    fn unregister_only_owner() {
        let r = ContentRendererRegistry::new();
        r.register(entry("mermaid", "acme.mermaid", &["text/x-mermaid"]))
            .unwrap();
        assert!(r.unregister("bob.y", "mermaid").is_err());
        assert!(r.unregister("acme.mermaid", "mermaid").unwrap());
        // Already removed → false (not an error).
        assert!(!r.unregister("acme.mermaid", "mermaid").unwrap());
    }

    #[test]
    fn unregister_all_for_drops_only_that_extensions() {
        let r = ContentRendererRegistry::new();
        r.register(entry("mermaid", "acme.mermaid", &["text/x-mermaid"]))
            .unwrap();
        r.register(entry("plantuml", "acme.mermaid", &["text/x-plantuml"]))
            .unwrap();
        r.register(entry("graphviz", "alice.gv", &["text/x-dot"]))
            .unwrap();
        let dropped = r.unregister_all_for("acme.mermaid");
        assert_eq!(dropped, 2);
        assert!(r.get("graphviz").is_some());
    }

    #[test]
    fn list_is_sorted() {
        let r = ContentRendererRegistry::new();
        r.register(entry("zeta", "alice.x", &["a/a"])).unwrap();
        r.register(entry("alpha", "alice.x", &["a/a"])).unwrap();
        r.register(entry("mu", "alice.x", &["a/a"])).unwrap();
        let ids: Vec<String> = r.list().into_iter().map(|e| e.renderer_id).collect();
        assert_eq!(ids, vec!["alpha", "mu", "zeta"]);
    }
}
