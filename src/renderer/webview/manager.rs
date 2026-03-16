//! Centralized z-order and layer tracker for multi-child webview windows.
//!
//! `WebviewManager` does NOT own the `BrowserView`s — those remain in the
//! `WebviewTab` vec in `AppState`.  Instead, this module tracks:
//!
//! - **Z-layer assignment**: which webviews are Docked, Overlay, or
//!   Independent.
//! - **Overlay z-stack**: the activation order of overlay/independent
//!   webviews so platform child windows can be re-stacked correctly.
//! - **Independent egui context**: optional egui state for independent
//!   overlay windows that render their own browser.

use std::collections::HashMap;

/// Unique webview identifier (same as WebviewTab::id in app.rs).
pub type WebviewId = u32;

/// The layer a webview occupies in the z-order stack.
///
/// Docked webviews are native child views of the main window and sit above
/// the wgpu surface (Metal / DX12 layer).  Overlay webviews live in
/// platform child windows (NSPanel / owned popup) that float ABOVE docked
/// views.  Independent overlays live in their own child window and may host
/// their own egui pass for rendering browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZLayer {
    /// Docked as a tile split — native child view of the main window.
    Docked,
    /// Floating overlay — lives in a child window above docked views.
    Overlay,
    /// Independent overlay — lives in its own child window, may have its
    /// own egui integration for rendering browser (address bar, etc.).
    Independent,
}

/// Per-webview metadata stored in the manager.
pub struct WebviewEntry {
    pub id: WebviewId,
    pub layer: ZLayer,
    /// For independent overlays: optional egui context for rendering
    /// UI browser within the child window.
    pub independent_egui: Option<IndependentEguiCtx>,
}

/// Egui context for an independent overlay window.
///
/// This allows rendering egui widgets (address bar, buttons) inside the
/// child window, on top of the webview.
#[allow(dead_code)]
#[derive(Default)]
pub struct IndependentEguiCtx {
    pub ctx: egui::Context,
    /// Address bar state for this independent overlay.
    pub address_bar_url: String,
    pub address_bar_editing: bool,
}

/// Lightweight z-order tracker for webview windows.
///
/// Works alongside the existing `webview_tabs: Vec<WebviewTab>` — does NOT
/// own `BrowserView` instances.  Register webview IDs after creating them,
/// and use the z-order API to manage overlay stacking.
#[derive(Default)]
pub struct WebviewManager {
    /// Per-webview metadata.
    entries: HashMap<WebviewId, WebviewEntry>,
    /// IDs of overlay/independent webviews sorted by activation order
    /// (most recently activated last = topmost).
    overlay_z_stack: Vec<WebviewId>,
}

impl WebviewManager {
    // ── Registration ────────────────────────────────────────────────────

    /// Register a webview ID with a given layer.
    ///
    /// Call this right after creating a `WebviewTab` so the manager can
    /// track its z-order.
    pub fn register(&mut self, id: WebviewId, layer: ZLayer) {
        let entry = WebviewEntry {
            id,
            layer,
            independent_egui: if layer == ZLayer::Independent {
                Some(IndependentEguiCtx::default())
            } else {
                None
            },
        };
        self.entries.insert(id, entry);
        if layer == ZLayer::Overlay || layer == ZLayer::Independent {
            self.overlay_z_stack.push(id);
        }
    }

    /// Unregister a webview ID.
    pub fn unregister(&mut self, id: WebviewId) {
        self.entries.remove(&id);
        self.overlay_z_stack.retain(|&wid| wid != id);
    }

    /// Get the layer for a webview.
    #[allow(dead_code)]
    pub fn layer(&self, id: WebviewId) -> Option<ZLayer> {
        self.entries.get(&id).map(|e| e.layer)
    }

    /// Get the entry for a webview.
    #[allow(dead_code)]
    pub fn get(&self, id: WebviewId) -> Option<&WebviewEntry> {
        self.entries.get(&id)
    }

    /// Get mutable entry.
    #[allow(dead_code)]
    pub fn get_mut(&mut self, id: WebviewId) -> Option<&mut WebviewEntry> {
        self.entries.get_mut(&id)
    }

    // ── Z-order management ──────────────────────────────────────────────

    /// Bring an overlay/independent webview to the top of the z-stack.
    ///
    /// Returns the new z-stack order so the caller can re-order
    /// platform child windows accordingly.
    pub fn bring_to_front(&mut self, id: WebviewId) -> &[WebviewId] {
        if let Some(entry) = self.entries.get(&id)
            && entry.layer == ZLayer::Docked
        {
            return &self.overlay_z_stack;
        }
        self.overlay_z_stack.retain(|&wid| wid != id);
        self.overlay_z_stack.push(id);
        &self.overlay_z_stack
    }

    /// Get the topmost overlay webview ID (if any are visible).
    #[allow(dead_code)]
    pub fn topmost_overlay(&self) -> Option<WebviewId> {
        self.overlay_z_stack.last().copied()
    }

    /// Get the overlay z-stack (bottom to top order).
    #[allow(dead_code)]
    pub fn overlay_stack(&self) -> &[WebviewId] {
        &self.overlay_z_stack
    }

    /// Remove a webview from the z-stack (e.g. when hiding it).
    #[allow(dead_code)]
    pub fn remove_from_z_stack(&mut self, id: WebviewId) {
        self.overlay_z_stack.retain(|&wid| wid != id);
    }

    /// Add to z-stack if not already present (e.g. when showing it).
    #[allow(dead_code)]
    pub fn add_to_z_stack(&mut self, id: WebviewId) {
        if let Some(entry) = self.entries.get(&id)
            && entry.layer != ZLayer::Docked
            && !self.overlay_z_stack.contains(&id)
        {
            self.overlay_z_stack.push(id);
        }
    }

    // ── Layer transitions ───────────────────────────────────────────────

    /// Promote a docked webview to an overlay (floating) layer.
    #[allow(dead_code)]
    pub fn promote_to_overlay(&mut self, id: WebviewId) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.layer = ZLayer::Overlay;
        }
        if !self.overlay_z_stack.contains(&id) {
            self.overlay_z_stack.push(id);
        }
    }

    /// Demote an overlay webview to a docked (tile split) layer.
    #[allow(dead_code)]
    pub fn demote_to_docked(&mut self, id: WebviewId) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.layer = ZLayer::Docked;
            entry.independent_egui = None;
        }
        self.overlay_z_stack.retain(|&wid| wid != id);
    }

    /// Promote an overlay to an independent overlay with its own egui context.
    #[allow(dead_code)]
    pub fn promote_to_independent(&mut self, id: WebviewId) {
        self.promote_to_independent_with_url(id, "");
    }

    /// Promote to independent and initialize the address bar with a URL.
    pub fn promote_to_independent_with_url(&mut self, id: WebviewId, url: &str) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.layer = ZLayer::Independent;
            if entry.independent_egui.is_none() {
                entry.independent_egui = Some(IndependentEguiCtx {
                    address_bar_url: url.to_string(),
                    ..IndependentEguiCtx::default()
                });
            }
        }
        if !self.overlay_z_stack.contains(&id) {
            self.overlay_z_stack.push(id);
        }
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// All docked webview IDs.
    #[allow(dead_code)]
    pub fn docked_ids(&self) -> Vec<WebviewId> {
        self.entries
            .values()
            .filter(|e| e.layer == ZLayer::Docked)
            .map(|e| e.id)
            .collect()
    }

    /// All overlay webview IDs (Overlay + Independent).
    #[allow(dead_code)]
    pub fn overlay_ids(&self) -> Vec<WebviewId> {
        self.entries
            .values()
            .filter(|e| e.layer == ZLayer::Overlay || e.layer == ZLayer::Independent)
            .map(|e| e.id)
            .collect()
    }

    /// All independent webview IDs.
    #[allow(dead_code)]
    pub fn independent_ids(&self) -> Vec<WebviewId> {
        self.entries
            .values()
            .filter(|e| e.layer == ZLayer::Independent)
            .map(|e| e.id)
            .collect()
    }

    /// Whether a webview is in the independent layer.
    #[allow(dead_code)]
    pub fn is_independent(&self, id: WebviewId) -> bool {
        self.entries
            .get(&id)
            .is_some_and(|e| e.layer == ZLayer::Independent)
    }

    /// Number of tracked webviews.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no tracked webviews.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
