//! `cronymax.ui.sidebar.view` runtime registry.
//!
//! Tracks sidebar webview contributions declared in
//! `contributes["cronymax.ui.sidebar.view"]` and activated at runtime by
//! the extension. Once Phase 6 wires the CEF iframe bridge, the platform
//! looks up entries here to spawn the actual panel.
//!
//! Sidebar entries deliberately live alongside other L2 EP registries
//! (`api::agents`, `api::renderers`) instead of being folded into
//! [`crate::extensions::api::webview::WebviewRegistry`]. WebviewRegistry
//! tracks **live panels** (visible / not, ownership for dispose); this
//! registry tracks **declared contributions** that the UI can offer to
//! the user even before the iframe is opened.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// One registered sidebar view. Pure metadata; UI routes
/// `view.show` / `view.hide` / `view.message` through
/// [`crate::extensions::runtime::ExtensionRuntime::notify_extension`].
#[derive(Debug, Clone)]
pub struct SidebarViewEntry {
    pub view_id: String,
    pub owning_ext: String,
    pub title: String,
    pub icon: Option<String>,
    /// Relative entry path inside the extension dir.
    pub entry: String,
}

#[derive(Debug, Default, Clone)]
pub struct SidebarViewRegistry {
    inner: Arc<RwLock<HashMap<String, SidebarViewEntry>>>,
}

impl SidebarViewRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, entry: SidebarViewEntry) -> ExtensionResult<()> {
        if entry.view_id.starts_with("cronymax.") || entry.view_id == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(format!(
                "sidebar view id `{}` is reserved by the platform",
                entry.view_id
            )));
        }
        let mut g = self.inner.write();
        if let Some(existing) = g.get(&entry.view_id) {
            if existing.owning_ext != entry.owning_ext {
                return Err(ExtensionError::BadContribution {
                    point: "cronymax.ui.sidebar.view".into(),
                    ext_id: entry.owning_ext.clone(),
                    reason: format!(
                        "sidebar view id `{}` is already owned by `{}`",
                        entry.view_id, existing.owning_ext
                    ),
                });
            }
        }
        g.insert(entry.view_id.clone(), entry);
        Ok(())
    }

    pub fn unregister(&self, ext_id: &str, view_id: &str) -> ExtensionResult<bool> {
        let mut g = self.inner.write();
        let Some(existing) = g.get(view_id) else {
            return Ok(false);
        };
        if existing.owning_ext != ext_id {
            return Err(ExtensionError::BadContribution {
                point: "cronymax.ui.sidebar.view".into(),
                ext_id: ext_id.to_string(),
                reason: format!(
                    "sidebar view `{view_id}` is owned by `{}`, not `{ext_id}`",
                    existing.owning_ext
                ),
            });
        }
        g.remove(view_id);
        Ok(true)
    }

    pub fn unregister_all_for(&self, ext_id: &str) -> usize {
        let mut g = self.inner.write();
        let before = g.len();
        g.retain(|_, e| e.owning_ext != ext_id);
        before - g.len()
    }

    pub fn get(&self, view_id: &str) -> Option<SidebarViewEntry> {
        self.inner.read().get(view_id).cloned()
    }

    pub fn list(&self) -> Vec<SidebarViewEntry> {
        let mut v: Vec<SidebarViewEntry> = self.inner.read().values().cloned().collect();
        v.sort_by(|a, b| a.view_id.cmp(&b.view_id));
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

    fn entry(view_id: &str, ext_id: &str) -> SidebarViewEntry {
        SidebarViewEntry {
            view_id: view_id.into(),
            owning_ext: ext_id.into(),
            title: "Title".into(),
            icon: None,
            entry: "./v.html".into(),
        }
    }

    #[test]
    fn register_then_lookup() {
        let r = SidebarViewRegistry::new();
        r.register(entry("view.main", "alice.x")).unwrap();
        assert_eq!(r.get("view.main").unwrap().owning_ext, "alice.x");
    }

    #[test]
    fn cronymax_namespace_rejected() {
        let r = SidebarViewRegistry::new();
        let err = r
            .register(entry("cronymax.builtin", "alice.x"))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NamespaceReserved(_)));
    }

    #[test]
    fn cross_extension_collision_rejected() {
        let r = SidebarViewRegistry::new();
        r.register(entry("shared", "alice.x")).unwrap();
        let err = r.register(entry("shared", "bob.y")).unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
    }

    #[test]
    fn unregister_only_owner() {
        let r = SidebarViewRegistry::new();
        r.register(entry("v1", "alice.x")).unwrap();
        assert!(r.unregister("bob.y", "v1").is_err());
        assert!(r.unregister("alice.x", "v1").unwrap());
    }

    #[test]
    fn unregister_all_for_scoped() {
        let r = SidebarViewRegistry::new();
        r.register(entry("a1", "alice.x")).unwrap();
        r.register(entry("a2", "alice.x")).unwrap();
        r.register(entry("b1", "bob.y")).unwrap();
        assert_eq!(r.unregister_all_for("alice.x"), 2);
        assert!(r.get("b1").is_some());
    }

    #[test]
    fn list_is_sorted() {
        let r = SidebarViewRegistry::new();
        r.register(entry("zeta", "alice.x")).unwrap();
        r.register(entry("alpha", "alice.x")).unwrap();
        let ids: Vec<String> = r.list().into_iter().map(|e| e.view_id).collect();
        assert_eq!(ids, vec!["alpha", "zeta"]);
    }
}
