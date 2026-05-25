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
use serde::{Deserialize, Serialize};

use crate::extensions::error::{ExtensionError, ExtensionResult};
use crate::extensions::manifest::RendererCsp;

/// Events the platform pushes to the renderer UI shell about the
/// content-renderer iframe lifecycle. Mirrors the wire-side
/// `RuntimeEventPayload::Raw` JSON the chat surface subscribes to via
/// the `extensions/renderer` topic.
///
/// v1 alpha only emits `HeightChanged`; chat-driven instance lifecycle
/// (create / update / dispose) is emitted from the chat dispatch site
/// (P6.5-T09) rather than this registry because the registry has no
/// notion of "active instances" — instances are per-block, not per
/// `(extension, renderer_id)`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RendererEvent {
    /// Content-renderer iframe reported its rendered height (px). The
    /// chat surface uses this to size the embedding `<iframe>` element,
    /// since cross-origin iframe content cannot be measured from the
    /// parent. Fires after every render and on internal layout changes
    /// the renderer observes (e.g. via `ResizeObserver`).
    //
    // NB: `rename_all = "camelCase"` at the enum level renames variant
    // tags (`HeightChanged` → `"heightChanged"`) but does NOT propagate
    // to the variant's fields. Per-variant `rename_all` is required so
    // `instance_id` ships on the wire as `instanceId`, matching the
    // rest of the cronymax wire conventions (extId, panelId, etc.).
    #[serde(rename_all = "camelCase")]
    HeightChanged { instance_id: String, px: i32 },
}

/// Callback installed by the composition root (see `runtime/services.rs`).
/// Cheap to clone (Arc), safe to call from any thread.
pub type RendererEventEmitter = Arc<dyn Fn(RendererEvent) + Send + Sync>;

/// One registered content renderer. Keyed in the registry by `renderer_id`.
#[derive(Debug, Clone)]
pub struct RendererEntry {
    pub renderer_id: String,
    pub owning_ext: String,
    /// MIME types this renderer claims, from the manifest.
    pub mime_types: Vec<String>,
    /// Entry path (relative to the extension dir) for the iframe.
    pub entry: String,
    /// Optional CSP overrides for the renderer iframe, mirroring
    /// [`crate::extensions::manifest::ContentRendererContribution::csp`].
    /// Consumed by the `cronymax-webview://` scheme handler when serving
    /// `?surface=renderer` responses (P6.5-T06).
    pub csp: Option<RendererCsp>,
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

    /// Ingest every `cronymax.content.renderer` contribution in `manifest`.
    /// Called once per extension at activate time — the registry is
    /// **manifest-driven** in v1 (post-P6.5 IDL): there is no Node-side
    /// `registerRenderer` API, so the only path into this registry is
    /// declarative.
    ///
    /// Idempotent: re-ingesting the same manifest (re-activate) does NOT
    /// produce duplicate entries because [`register`] returns Ok when the
    /// same `(ext_id, renderer_id)` pair is re-claimed.
    ///
    /// Returns the number of renderers ingested. Errors only on registry
    /// collisions (e.g. another extension already owns the same id) or
    /// reserved-namespace violations — manifest validation should have
    /// caught those upstream, so a failure here means a real bug.
    pub fn ingest_manifest(
        &self,
        manifest: &crate::extensions::manifest::Manifest,
        ext_id: &str,
    ) -> ExtensionResult<usize> {
        let mut n = 0;
        for decl in &manifest.contributes.content_renderers {
            self.register(RendererEntry {
                renderer_id: decl.id.clone(),
                owning_ext: ext_id.to_string(),
                mime_types: decl.mime_types.clone(),
                entry: decl.entry.clone(),
                csp: decl.csp.clone(),
            })?;
            n += 1;
        }
        Ok(n)
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
            csp: None,
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

    /// `ingest_manifest` copies the declared `csp` field through to the
    /// `RendererEntry`. The scheme handler (P6.5-T06) reads this to merge
    /// host allowlists into the iframe's `connect-src` CSP header.
    #[test]
    fn ingest_manifest_propagates_csp_connect_src() {
        let raw = r#"{
            "id": "acme.diagrams",
            "name": "Diagrams",
            "version": "0.1.0",
            "publisher": "acme",
            "engines": { "cronymax": "^1.0" },
            "activationEvents": [],
            "contributes": {
                "cronymax.content.renderer": [
                    {
                        "id": "acme.diagrams.mermaid",
                        "mimeTypes": ["text/vnd.mermaid"],
                        "entry": "./r.html",
                        "csp": { "connect_src": ["https://mermaid.ink"] }
                    },
                    {
                        "id": "acme.diagrams.plain",
                        "mimeTypes": ["text/x-plain"],
                        "entry": "./p.html"
                    }
                ]
            }
        }"#;
        let manifest = crate::extensions::manifest::Manifest::from_json(raw).unwrap();
        let r = ContentRendererRegistry::new();
        let n = r.ingest_manifest(&manifest, "acme.diagrams").unwrap();
        assert_eq!(n, 2);

        let mermaid = r.get("acme.diagrams.mermaid").unwrap();
        assert_eq!(
            mermaid.csp.as_ref().unwrap().connect_src,
            vec!["https://mermaid.ink".to_string()],
        );

        // Renderer without `csp` carries `None`, NOT an empty struct —
        // distinguishing "no override" from "empty allowlist" matters for
        // the scheme handler's CSP header serialiser.
        let plain = r.get("acme.diagrams.plain").unwrap();
        assert!(plain.csp.is_none());
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
