//! `window.createWebviewPanel` and panel-message routing.
//!
//! This module owns **the platform-side panel registry**: which panels
//! exist, who owns them, and what their visibility state is. The actual
//! rendering (CEF custom protocol handler + iframe sandbox + postMessage
//! bridge) lands in Phase 6.
//!
//! Mirrors `cep-idl/v1/window.ts` panel-related types.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// Where a panel renders. Mirrors the `UiSlot` IDL string union.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelSlot {
    Sidebar,
    Settings,
    ActivityBar,
    StatusBar,
}

impl PanelSlot {
    pub fn from_idl_str(raw: &str) -> Option<Self> {
        Some(match raw {
            "sidebar" => Self::Sidebar,
            "settings" => Self::Settings,
            "activitybar" => Self::ActivityBar,
            "statusbar" => Self::StatusBar,
            _ => return None,
        })
    }

    pub fn idl_str(&self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Settings => "settings",
            Self::ActivityBar => "activitybar",
            Self::StatusBar => "statusbar",
        }
    }
}

/// Args passed to `window.createWebviewPanel`. Mirrors the IDL.
#[derive(Clone, Debug)]
pub struct CreatePanelArgs {
    pub ext_id: String,
    pub panel_id: String,
    pub title: String,
    pub slot: PanelSlot,
    /// Entry path relative to the extension's root, served by the CEF
    /// custom protocol handler in Phase 6.
    pub entry: String,
}

/// One live webview panel as the platform sees it. P6 augments this with
/// the CEF browser id and the postMessage bridge state; v1 alpha tracks
/// only the metadata necessary to enforce ownership and avoid id clashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelView {
    pub ext_id: String,
    pub panel_id: String,
    pub title: String,
    pub slot: PanelSlot,
    pub entry: String,
    pub visible: bool,
}

/// Process-wide registry of webview panels. Cheap to share.
#[derive(Debug, Default)]
pub struct WebviewRegistry {
    panels: Mutex<HashMap<String, PanelView>>,
}

impl WebviewRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a panel. Errors if `panel_id` is already taken by any
    /// extension (panel ids share a global namespace because the user-
    /// visible "show this panel" command keys off the id).
    pub fn create(&self, args: CreatePanelArgs) -> ExtensionResult<PanelView> {
        let mut g = self
            .panels
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("webview registry poisoned".into()))?;
        if g.contains_key(&args.panel_id) {
            return Err(ExtensionError::BadContribution {
                point: "cronymax.window.panel".into(),
                ext_id: args.ext_id.clone(),
                reason: format!("panel `{}` already exists", args.panel_id),
            });
        }
        let view = PanelView {
            ext_id: args.ext_id,
            panel_id: args.panel_id.clone(),
            title: args.title,
            slot: args.slot,
            entry: args.entry,
            visible: false,
        };
        g.insert(args.panel_id, view.clone());
        Ok(view)
    }

    /// Drop a panel. Returns true if it was present. Errors if a
    /// different extension owns the panel (defence in depth — the RPC
    /// layer also checks).
    pub fn dispose(&self, ext_id: &str, panel_id: &str) -> ExtensionResult<bool> {
        let mut g = self
            .panels
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("webview registry poisoned".into()))?;
        match g.get(panel_id) {
            Some(p) if p.ext_id == ext_id => {
                g.remove(panel_id);
                Ok(true)
            }
            Some(p) => Err(ExtensionError::BadContribution {
                point: "cronymax.window.panel".into(),
                ext_id: ext_id.to_string(),
                reason: format!(
                    "panel `{panel_id}` is owned by `{}`, not `{ext_id}`",
                    p.ext_id
                ),
            }),
            None => Ok(false),
        }
    }

    /// Drop every panel owned by `ext_id`. Used on deactivate.
    pub fn dispose_all_for(&self, ext_id: &str) -> ExtensionResult<usize> {
        let mut g = self
            .panels
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("webview registry poisoned".into()))?;
        let before = g.len();
        g.retain(|_, p| p.ext_id != ext_id);
        Ok(before - g.len())
    }

    pub fn set_visible(&self, ext_id: &str, panel_id: &str, visible: bool) -> ExtensionResult<()> {
        let mut g = self
            .panels
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("webview registry poisoned".into()))?;
        let panel = g
            .get_mut(panel_id)
            .ok_or_else(|| ExtensionError::BadContribution {
                point: "cronymax.window.panel".into(),
                ext_id: ext_id.to_string(),
                reason: format!("panel `{panel_id}` does not exist"),
            })?;
        if panel.ext_id != ext_id {
            return Err(ExtensionError::BadContribution {
                point: "cronymax.window.panel".into(),
                ext_id: ext_id.to_string(),
                reason: format!(
                    "panel `{panel_id}` is owned by `{}`, not `{ext_id}`",
                    panel.ext_id
                ),
            });
        }
        panel.visible = visible;
        Ok(())
    }

    pub fn get(&self, panel_id: &str) -> ExtensionResult<Option<PanelView>> {
        let g = self
            .panels
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("webview registry poisoned".into()))?;
        Ok(g.get(panel_id).cloned())
    }

    pub fn list_for(&self, ext_id: &str) -> ExtensionResult<Vec<PanelView>> {
        let g = self
            .panels
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("webview registry poisoned".into()))?;
        let mut v: Vec<PanelView> = g.values().filter(|p| p.ext_id == ext_id).cloned().collect();
        v.sort_by(|a, b| a.panel_id.cmp(&b.panel_id));
        Ok(v)
    }

    pub fn len(&self) -> usize {
        self.panels.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(panel_id: &str, ext_id: &str) -> CreatePanelArgs {
        CreatePanelArgs {
            ext_id: ext_id.into(),
            panel_id: panel_id.into(),
            title: "Title".into(),
            slot: PanelSlot::Sidebar,
            entry: "./panel.html".into(),
        }
    }

    #[test]
    fn slot_idl_round_trip() {
        for slot in [
            PanelSlot::Sidebar,
            PanelSlot::Settings,
            PanelSlot::ActivityBar,
            PanelSlot::StatusBar,
        ] {
            assert_eq!(PanelSlot::from_idl_str(slot.idl_str()), Some(slot));
        }
        assert_eq!(PanelSlot::from_idl_str("unknown"), None);
    }

    #[test]
    fn create_then_lookup_round_trip() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let p = r.get("p1").unwrap().unwrap();
        assert_eq!(p.ext_id, "alice.x");
        assert_eq!(p.title, "Title");
        assert!(!p.visible);
    }

    #[test]
    fn duplicate_panel_id_errors() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let err = r.create(args("p1", "bob.y")).unwrap_err();
        assert!(
            matches!(err, ExtensionError::BadContribution { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn dispose_only_works_for_owner() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let err = r.dispose("bob.y", "p1").unwrap_err();
        assert!(
            matches!(err, ExtensionError::BadContribution { .. }),
            "got {err:?}"
        );
        // Owner can dispose
        assert!(r.dispose("alice.x", "p1").unwrap());
        // Second dispose: returns false (already gone)
        assert!(!r.dispose("alice.x", "p1").unwrap());
    }

    #[test]
    fn dispose_all_for_drops_only_that_extensions_panels() {
        let r = WebviewRegistry::new();
        r.create(args("a1", "alice.x")).unwrap();
        r.create(args("a2", "alice.x")).unwrap();
        r.create(args("b1", "bob.y")).unwrap();
        let dropped = r.dispose_all_for("alice.x").unwrap();
        assert_eq!(dropped, 2);
        assert!(r.get("a1").unwrap().is_none());
        assert!(r.get("b1").unwrap().is_some());
    }

    #[test]
    fn set_visible_round_trip() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        r.set_visible("alice.x", "p1", true).unwrap();
        assert!(r.get("p1").unwrap().unwrap().visible);
        r.set_visible("alice.x", "p1", false).unwrap();
        assert!(!r.get("p1").unwrap().unwrap().visible);
    }

    #[test]
    fn set_visible_other_extension_errors() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let err = r.set_visible("bob.y", "p1", true).unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
    }

    #[test]
    fn set_visible_unknown_panel_errors() {
        let r = WebviewRegistry::new();
        let err = r.set_visible("alice.x", "ghost", true).unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
    }

    #[test]
    fn list_for_is_sorted_and_scoped() {
        let r = WebviewRegistry::new();
        r.create(args("zeta", "alice.x")).unwrap();
        r.create(args("alpha", "alice.x")).unwrap();
        r.create(args("beta", "bob.y")).unwrap();
        let ids: Vec<String> = r
            .list_for("alice.x")
            .unwrap()
            .into_iter()
            .map(|p| p.panel_id)
            .collect();
        assert_eq!(ids, vec!["alpha", "zeta"]);
    }
}
