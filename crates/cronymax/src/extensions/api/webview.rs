//! `window.createWebviewPanel` and panel-message routing.
//!
//! This module owns **the platform-side panel registry**: which panels
//! exist, who owns them, and what their visibility state is. The actual
//! rendering (CEF custom protocol handler + iframe sandbox + postMessage
//! bridge) lives in the renderer; this module is the source of truth the
//! renderer subscribes to via the [`WebviewEvent`] stream.
//!
//! Mirrors `cep-idl/v1/window.ts` panel-related types.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// Where a panel renders. Mirrors the `WebviewSlot` IDL string union in
/// `cep-idl/v1/window.ts`. The platform additionally accepts two legacy
/// slot names (`activitybar` / `statusbar`) that predate the IDL freeze;
/// new extensions should use one of the three IDL slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelSlot {
    /// IDL `sidebar` — appears in the sidebar nav slot.
    Sidebar,
    /// IDL `settings` — appears as a settings-panel sub-tab.
    Settings,
    /// IDL `tab` — appears as a top-level main-area tab.
    Tab,
    /// Legacy: activity-bar slot, not in IDL.
    ActivityBar,
    /// Legacy: status-bar slot, not in IDL.
    StatusBar,
}

impl PanelSlot {
    pub fn from_idl_str(raw: &str) -> Option<Self> {
        Some(match raw {
            "sidebar" => Self::Sidebar,
            "settings" => Self::Settings,
            "tab" => Self::Tab,
            "activitybar" => Self::ActivityBar,
            "statusbar" => Self::StatusBar,
            _ => return None,
        })
    }

    pub fn idl_str(&self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Settings => "settings",
            Self::Tab => "tab",
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

/// Events the platform pushes to the renderer about the panel registry.
/// One emitter callback is wired at composition root (see
/// `runtime/services.rs`); it bridges into the authority's event topic
/// `extensions/webview` so any panel UI shell can subscribe.
///
/// The event shape is mirrored on the wire as a `RuntimeEventPayload::Raw`
/// JSON object — keeping it `serde`-serializable here means renderer code
/// can typecheck the same shape via `cep-idl/v1` codegen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WebviewEvent {
    /// A new panel was created. Renderer mounts an iframe at `url`
    /// (`cronymax-webview://<extId>/<entry>`) into the named `slot`.
    PanelCreated {
        ext_id: String,
        panel_id: String,
        title: String,
        slot: String,
        entry: String,
        /// Pre-built `cronymax-webview://<extId>/<entry>` URL the iframe
        /// loads. Keeping it server-side prevents the renderer from
        /// guessing path encoding rules.
        url: String,
    },
    /// Owner extension (or the platform via deactivate cleanup) asked
    /// the panel be torn down. Renderer unmounts the iframe.
    PanelDisposed { panel_id: String },
    /// Extension → iframe `postMessage`. Renderer forwards to the
    /// iframe's `acquireCronymaxApi().onDidReceiveMessage` listeners.
    Message { panel_id: String, payload: Value },
    /// Extension toggled visibility. Renderer adjusts CSS / mounts to
    /// the corresponding slot.
    VisibilityChanged { panel_id: String, visible: bool },
}

/// Callback installed by the composition root. Cheap to clone (Arc),
/// safe to call from any thread. The default is a no-op so a runtime
/// constructed without a renderer (e.g. unit tests) doesn't panic on
/// emit.
pub type WebviewEventEmitter = Arc<dyn Fn(WebviewEvent) + Send + Sync>;

/// Default emitter — silently drops events. Replaced via
/// [`WebviewRegistry::set_emitter`] at composition root.
fn noop_emitter() -> WebviewEventEmitter {
    Arc::new(|_| {})
}

/// Process-wide registry of webview panels. Cheap to share (`Arc`
/// internals); keep one instance per process.
#[derive(Clone, Debug)]
pub struct WebviewRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    panels: Mutex<HashMap<String, PanelView>>,
    /// Composition-root callback used to surface
    /// [`WebviewEvent`] payloads to whatever transport the renderer is
    /// listening on. `RwLock` so `set_emitter` can swap it without
    /// blocking the hot emit path (which acquires read).
    emitter: RwLock<WebviewEventEmitter>,
}

impl std::fmt::Debug for RegistryInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryInner")
            .field("panels", &self.panels)
            .field("emitter", &"<fn>")
            .finish()
    }
}

impl Default for WebviewRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                panels: Mutex::new(HashMap::new()),
                emitter: RwLock::new(noop_emitter()),
            }),
        }
    }
}

impl WebviewRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install / replace the renderer-facing event emitter. Called once
    /// at composition root (see `runtime/services.rs`). Subsequent
    /// [`create`] / [`dispose`] / [`set_visible`] / [`deliver_message`]
    /// calls fire through this closure.
    pub fn set_emitter(&self, emitter: WebviewEventEmitter) {
        if let Ok(mut g) = self.inner.emitter.write() {
            *g = emitter;
        }
    }

    /// Build the canonical `cronymax-webview://<extId>/<entry>` URL that
    /// the renderer mounts into an iframe. Path-normalises leading `./`
    /// from manifest-declared entries (`./panel.html` → `panel.html`).
    ///
    /// When the iframe's JS calls `acquireCronymaxApi()` it needs to know
    /// which panel it belongs to (one extension can mount the same entry
    /// into multiple panels). The platform identifies the iframe surface
    /// with a two-key query: `?surface=<kind>&id=<value>`, where `<kind>`
    /// is `panel` for [`createWebviewPanel`] / [`PanelView`] iframes and
    /// `renderer` for [`ContentRendererContribution`] iframes (see
    /// `renderer-host.ts`). The V8 injection layer reads both keys
    /// straight out of `frame->GetURL()` and chooses which SDK to inject.
    ///
    /// Old single-key `?panel=<id>` URLs are no longer emitted; the C++
    /// scheme handler and V8 injection only understand the surfaced form.
    pub fn url_for(ext_id: &str, panel_id: &str, entry: &str) -> String {
        Self::url_for_surface(ext_id, "panel", panel_id, entry)
    }

    /// Build a `cronymax-webview://` URL for any surface kind. v1 surfaces:
    ///   * `panel`    — webview panels (window.createWebviewPanel)
    ///   * `renderer` — content renderer iframes (P6.5)
    ///
    /// The `id` is panel id (for panel surface) or renderer instance id
    /// (for renderer surface). It is percent-encoded for the same minimal
    /// set as [`url_for`].
    pub fn url_for_surface(ext_id: &str, surface: &str, id: &str, entry: &str) -> String {
        let cleaned = entry.trim_start_matches("./").trim_start_matches('/');
        // ext_id, surface and id are platform / manifest identifiers, but
        // defensively escape the few characters that would break URL
        // parsing. Most ids are dotted (`alice.x.panel.foo`) and need no
        // escaping at all.
        fn escape(s: &str) -> String {
            let mut out = String::with_capacity(s.len());
            for ch in s.chars() {
                match ch {
                    '?' | '#' | '&' | '=' | ' ' | '/' | '\\' => {
                        out.push_str(&format!("%{:02X}", ch as u32));
                    }
                    _ => out.push(ch),
                }
            }
            out
        }
        format!(
            "cronymax-webview://{ext_id}/{cleaned}?surface={s}&id={i}",
            s = escape(surface),
            i = escape(id),
        )
    }

    fn lock_panels(
        &self,
    ) -> ExtensionResult<std::sync::MutexGuard<'_, HashMap<String, PanelView>>> {
        self.inner
            .panels
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("webview registry poisoned".into()))
    }

    fn emit(&self, event: WebviewEvent) {
        // Clone the emitter under read lock then drop the lock before
        // firing — emitters might re-enter the registry (e.g. to read
        // the panel they just saw).
        let emitter = match self.inner.emitter.read() {
            Ok(g) => g.clone(),
            Err(_) => return,
        };
        emitter(event);
    }

    /// Create a panel. Errors if `panel_id` is already taken by any
    /// extension (panel ids share a global namespace because the user-
    /// visible "show this panel" command keys off the id). Fires
    /// [`WebviewEvent::PanelCreated`] on success.
    pub fn create(&self, args: CreatePanelArgs) -> ExtensionResult<PanelView> {
        let view = {
            let mut g = self.lock_panels()?;
            if g.contains_key(&args.panel_id) {
                return Err(ExtensionError::BadContribution {
                    point: "cronymax.window.panel".into(),
                    ext_id: args.ext_id.clone(),
                    reason: format!("panel `{}` already exists", args.panel_id),
                });
            }
            let view = PanelView {
                ext_id: args.ext_id.clone(),
                panel_id: args.panel_id.clone(),
                title: args.title.clone(),
                slot: args.slot,
                entry: args.entry.clone(),
                visible: false,
            };
            g.insert(args.panel_id.clone(), view.clone());
            view
        };
        self.emit(WebviewEvent::PanelCreated {
            ext_id: view.ext_id.clone(),
            panel_id: view.panel_id.clone(),
            title: view.title.clone(),
            slot: view.slot.idl_str().to_string(),
            entry: view.entry.clone(),
            url: Self::url_for(&view.ext_id, &view.panel_id, &view.entry),
        });
        Ok(view)
    }

    /// Drop a panel. Returns true if it was present. Errors if a
    /// different extension owns the panel (defence in depth — the RPC
    /// layer also checks). Fires [`WebviewEvent::PanelDisposed`] when
    /// the panel actually went away.
    pub fn dispose(&self, ext_id: &str, panel_id: &str) -> ExtensionResult<bool> {
        let removed = {
            let mut g = self.lock_panels()?;
            match g.get(panel_id) {
                Some(p) if p.ext_id == ext_id => {
                    g.remove(panel_id);
                    true
                }
                Some(p) => {
                    return Err(ExtensionError::BadContribution {
                        point: "cronymax.window.panel".into(),
                        ext_id: ext_id.to_string(),
                        reason: format!(
                            "panel `{panel_id}` is owned by `{}`, not `{ext_id}`",
                            p.ext_id
                        ),
                    })
                }
                None => false,
            }
        };
        if removed {
            self.emit(WebviewEvent::PanelDisposed {
                panel_id: panel_id.to_string(),
            });
        }
        Ok(removed)
    }

    /// Drop every panel owned by `ext_id`. Used on deactivate. Fires one
    /// `PanelDisposed` per removed panel.
    pub fn dispose_all_for(&self, ext_id: &str) -> ExtensionResult<usize> {
        let removed_ids: Vec<String> = {
            let mut g = self.lock_panels()?;
            let removed: Vec<String> = g
                .iter()
                .filter(|(_, p)| p.ext_id == ext_id)
                .map(|(k, _)| k.clone())
                .collect();
            for id in &removed {
                g.remove(id);
            }
            removed
        };
        for panel_id in &removed_ids {
            self.emit(WebviewEvent::PanelDisposed {
                panel_id: panel_id.clone(),
            });
        }
        Ok(removed_ids.len())
    }

    pub fn set_visible(&self, ext_id: &str, panel_id: &str, visible: bool) -> ExtensionResult<()> {
        {
            let mut g = self.lock_panels()?;
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
        }
        self.emit(WebviewEvent::VisibilityChanged {
            panel_id: panel_id.to_string(),
            visible,
        });
        Ok(())
    }

    /// Forward an extension → iframe message. Validates ownership, then
    /// fires [`WebviewEvent::Message`] so the renderer can route the
    /// payload into the right iframe's `acquireCronymaxApi().onDidReceiveMessage`.
    pub fn deliver_message(
        &self,
        ext_id: &str,
        panel_id: &str,
        payload: Value,
    ) -> ExtensionResult<()> {
        {
            let g = self.lock_panels()?;
            let panel = g
                .get(panel_id)
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
        }
        self.emit(WebviewEvent::Message {
            panel_id: panel_id.to_string(),
            payload,
        });
        Ok(())
    }

    /// Look up the owning extension of a panel — used by the cefQuery
    /// dispatch in the C++ renderer-side bridge to route iframe →
    /// extension `postMessage` to the right Node host.
    pub fn owner_of(&self, panel_id: &str) -> ExtensionResult<Option<String>> {
        let g = self.lock_panels()?;
        Ok(g.get(panel_id).map(|p| p.ext_id.clone()))
    }

    pub fn get(&self, panel_id: &str) -> ExtensionResult<Option<PanelView>> {
        let g = self.lock_panels()?;
        Ok(g.get(panel_id).cloned())
    }

    pub fn list_for(&self, ext_id: &str) -> ExtensionResult<Vec<PanelView>> {
        let g = self.lock_panels()?;
        let mut v: Vec<PanelView> = g.values().filter(|p| p.ext_id == ext_id).cloned().collect();
        v.sort_by(|a, b| a.panel_id.cmp(&b.panel_id));
        Ok(v)
    }

    /// Snapshot every live panel. Useful for the renderer's initial
    /// mount: subscribe to events, then drain the snapshot so it can
    /// mount iframes for panels created before the subscription.
    pub fn snapshot(&self) -> ExtensionResult<Vec<PanelView>> {
        let g = self.lock_panels()?;
        let mut v: Vec<PanelView> = g.values().cloned().collect();
        v.sort_by(|a, b| a.panel_id.cmp(&b.panel_id));
        Ok(v)
    }

    pub fn len(&self) -> usize {
        self.inner.panels.lock().map(|g| g.len()).unwrap_or(0)
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
            PanelSlot::Tab,
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

    #[test]
    fn url_for_strips_relative_prefix_and_appends_surface_and_id() {
        assert_eq!(
            WebviewRegistry::url_for("alice.x", "p1", "./panel.html"),
            "cronymax-webview://alice.x/panel.html?surface=panel&id=p1",
        );
        assert_eq!(
            WebviewRegistry::url_for("alice.x", "deep.panel", "/abs/index.html"),
            "cronymax-webview://alice.x/abs/index.html?surface=panel&id=deep.panel",
        );
        assert_eq!(
            WebviewRegistry::url_for("alice.x", "nested.p", "deep/nested/index.html"),
            "cronymax-webview://alice.x/deep/nested/index.html?surface=panel&id=nested.p",
        );
    }

    #[test]
    fn url_for_escapes_reserved_chars_in_id() {
        // The id field is user-controlled (extension manifest / runtime);
        // a stray `?`, `&`, or `=` would corrupt the URL parse. We
        // percent-encode the small reserved set.
        assert_eq!(
            WebviewRegistry::url_for("alice.x", "weird?panel&id", "p.html"),
            "cronymax-webview://alice.x/p.html?surface=panel&id=weird%3Fpanel%26id",
        );
    }

    #[test]
    fn url_for_surface_renderer_emits_renderer_keyword() {
        // P6.5 content renderer iframes go through the same helper but
        // emit `surface=renderer` so the V8 layer injects
        // `acquireCronymaxRendererApi()` instead of `acquireCronymaxApi()`.
        assert_eq!(
            WebviewRegistry::url_for_surface(
                "acme.mermaid",
                "renderer",
                "instance-42",
                "renderer/index.html",
            ),
            "cronymax-webview://acme.mermaid/renderer/index.html?surface=renderer&id=instance-42",
        );
    }

    #[test]
    fn owner_of_round_trips() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        assert_eq!(r.owner_of("p1").unwrap().as_deref(), Some("alice.x"));
        assert_eq!(r.owner_of("ghost").unwrap(), None);
    }

    #[test]
    fn snapshot_returns_every_panel_sorted() {
        let r = WebviewRegistry::new();
        r.create(args("zeta", "alice.x")).unwrap();
        r.create(args("alpha", "bob.y")).unwrap();
        let ids: Vec<String> = r
            .snapshot()
            .unwrap()
            .into_iter()
            .map(|p| p.panel_id)
            .collect();
        assert_eq!(ids, vec!["alpha", "zeta"]);
    }

    fn collect_emitter() -> (Arc<Mutex<Vec<WebviewEvent>>>, WebviewEventEmitter) {
        let log: Arc<Mutex<Vec<WebviewEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let cap = log.clone();
        let emitter: WebviewEventEmitter = Arc::new(move |ev| {
            cap.lock().unwrap().push(ev);
        });
        (log, emitter)
    }

    #[test]
    fn create_fires_panel_created_event_with_url() {
        let r = WebviewRegistry::new();
        let (log, em) = collect_emitter();
        r.set_emitter(em);
        r.create(args("p1", "alice.x")).unwrap();
        let events = log.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        match &events[0] {
            WebviewEvent::PanelCreated {
                ext_id,
                panel_id,
                slot,
                url,
                ..
            } => {
                assert_eq!(ext_id, "alice.x");
                assert_eq!(panel_id, "p1");
                assert_eq!(slot, "sidebar");
                assert_eq!(
                    url,
                    "cronymax-webview://alice.x/panel.html?surface=panel&id=p1"
                );
            }
            other => panic!("expected PanelCreated, got {other:?}"),
        }
    }

    #[test]
    fn dispose_fires_panel_disposed_event() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let (log, em) = collect_emitter();
        r.set_emitter(em);
        r.dispose("alice.x", "p1").unwrap();
        let events = log.lock().unwrap().clone();
        assert!(matches!(
            events.as_slice(),
            [WebviewEvent::PanelDisposed { panel_id }] if panel_id == "p1"
        ));
    }

    #[test]
    fn dispose_for_missing_panel_does_not_fire_event() {
        let r = WebviewRegistry::new();
        let (log, em) = collect_emitter();
        r.set_emitter(em);
        let removed = r.dispose("alice.x", "ghost").unwrap();
        assert!(!removed);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn dispose_all_for_fires_one_event_per_removed_panel() {
        let r = WebviewRegistry::new();
        r.create(args("a1", "alice.x")).unwrap();
        r.create(args("a2", "alice.x")).unwrap();
        r.create(args("b1", "bob.y")).unwrap();
        let (log, em) = collect_emitter();
        r.set_emitter(em);
        let removed = r.dispose_all_for("alice.x").unwrap();
        assert_eq!(removed, 2);
        let events = log.lock().unwrap().clone();
        let ids: std::collections::HashSet<String> = events
            .into_iter()
            .filter_map(|e| match e {
                WebviewEvent::PanelDisposed { panel_id } => Some(panel_id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, ["a1", "a2"].iter().map(|s| s.to_string()).collect());
    }

    #[test]
    fn set_visible_fires_visibility_changed_event() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let (log, em) = collect_emitter();
        r.set_emitter(em);
        r.set_visible("alice.x", "p1", true).unwrap();
        let events = log.lock().unwrap().clone();
        assert!(matches!(
            events.as_slice(),
            [WebviewEvent::VisibilityChanged { panel_id, visible }]
                if panel_id == "p1" && *visible
        ));
    }

    #[test]
    fn deliver_message_fires_message_event_for_owner() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let (log, em) = collect_emitter();
        r.set_emitter(em);
        r.deliver_message("alice.x", "p1", serde_json::json!({"hi": "there"}))
            .unwrap();
        let events = log.lock().unwrap().clone();
        match &events[..] {
            [WebviewEvent::Message { panel_id, payload }] => {
                assert_eq!(panel_id, "p1");
                assert_eq!(payload.get("hi").and_then(|v| v.as_str()), Some("there"),);
            }
            other => panic!("expected one Message event, got {other:?}"),
        }
    }

    #[test]
    fn deliver_message_from_non_owner_errors_and_does_not_emit() {
        let r = WebviewRegistry::new();
        r.create(args("p1", "alice.x")).unwrap();
        let (log, em) = collect_emitter();
        r.set_emitter(em);
        let err = r
            .deliver_message("bob.y", "p1", serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
        assert!(
            log.lock().unwrap().is_empty(),
            "no event on rejected delivery"
        );
    }

    #[test]
    fn deliver_message_to_missing_panel_errors() {
        let r = WebviewRegistry::new();
        let err = r
            .deliver_message("alice.x", "ghost", serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
    }
}
