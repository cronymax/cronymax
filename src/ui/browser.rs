//! Browser model, view widget, and overlay positioning.
//!
//! - [`Browser`]: converged model owning the native webview + `AddressBarState`
//! - [`BrowserView`]: unified address-bar widget (container-agnostic)
//! - [`BrowserOverlay`]: overlay sizing/positioning on the main egui surface
//! - [`BrowserManager`]: z-order tracker for browser windows
//! - [`BrowserTab`]: per-browser-tab state
//! - [`ZLayer`] / [`BrowserEntry`]: z-layer metadata

use std::collections::HashMap;

use super::actions::UiAction;
use super::i18n::t;
use super::icons::{Icon, IconButtonCfg, icon_button};
use super::styles::Styles;
use super::types::AddressBarState;

use crate::renderer::terminal::SessionId;
use crate::renderer::viewport::Viewport;
use crate::renderer::webview::Webview;

// ─── Browser Model ──────────────────────────────────────────────────────────

/// Converged browser model: owns the native webview delegate (`BrowserView`)
/// and the egui address-bar state.  Container-agnostic — works the same
/// whether hosted in a docked tile pane or a floating `Modal` overlay.
pub struct Browser {
    pub id: u32,
    pub title: String,
    pub url: String,
    pub view: Webview,
    pub address_bar: AddressBarState,
}

impl Browser {
    /// Create a new browser, constructing the underlying wry webview
    /// as a child of `parent`.
    pub fn new(
        id: u32,
        parent: &impl raw_window_handle::HasWindowHandle,
        url: &str,
        viewport: Viewport,
    ) -> Result<Self, String> {
        let view = Webview::new(parent, url, viewport)?;
        Ok(Self {
            id,
            title: String::new(),
            url: url.to_string(),
            view,
            address_bar: AddressBarState::new(url),
        })
    }

    // ── Navigation ──────────────────────────────────────────────────────

    /// Navigate to `url`, updating all internal state.
    pub fn navigate(&mut self, url: &str) {
        self.view.navigate(url);
        self.url = url.to_string();
        self.address_bar.update_url(url);
    }

    pub fn go_back(&self) {
        let _ = self.view.webview.evaluate_script("window.history.back()");
    }

    pub fn go_forward(&self) {
        let _ = self
            .view
            .webview
            .evaluate_script("window.history.forward()");
    }

    pub fn refresh(&self) {
        self.view.navigate(&self.url);
    }

    // ── Per-frame sync ──────────────────────────────────────────────────

    /// Drain navigated-URL changes from the webview and update both
    /// `self.url` and `self.address_bar`.
    /// Returns the new URL if one was received (for tile-tree sync).
    pub fn sync_nav_url(&mut self) -> Option<String> {
        if let Some(nav_url) = self.view.drain_nav_url() {
            self.url = nav_url.clone();
            self.address_bar.update_url(&nav_url);
            Some(nav_url)
        } else {
            None
        }
    }

    /// Drain document-title changes.
    /// Returns the new title if one was received.
    pub fn sync_title(&mut self) -> Option<String> {
        if let Some(new_title) = self.view.drain_title_change() {
            self.title = new_title.clone();
            Some(new_title)
        } else {
            None
        }
    }
}

// ─── BrowserView (unified address bar widget) ─────────────────────────────

use crate::ui::widget::{Fragment, Widget};

/// Unified browser‐view widget: address bar (navigation + URL + action buttons).
///
/// Container‐agnostic — works the same whether drawn inside a docked tile
/// pane or a floating `Modal` overlay.  Callers provide a `&mut egui::Ui`
/// that is already in a horizontal layout with the desired bar height.
pub struct BrowserView<'a> {
    pub webview_id: u32,
    pub url: &'a mut String,
    pub editing: &'a mut bool,
    /// When true, right-side action buttons are hidden (shown in tab bar instead).
    pub docked: bool,
}

impl BrowserView<'_> {
    fn draw<'a>(&mut self, ui: &'a mut egui::Ui, mut ctx: super::widget::Context<'a>) {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ctx.styles.address_bar_height()),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(ctx.styles.spacing.medium as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_centered(|ui| {
                            ctx.bind::<egui::Ui>(ui).add(AddressBarWidget {
                                close_webview_id: self.webview_id,
                                editing: self.editing,
                                url: self.url,
                                docked: self.docked,
                            })
                        });
                    });
            },
        );
    }
}

impl Widget<egui::Context> for BrowserView<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Context as super::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: super::widget::Context<'a>,
    ) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ctx.colors.bg_float)
                    .corner_radius(egui::CornerRadius::same(ctx.styles.radii.md as u8))
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ui, |ui| {
                self.draw(ui, ctx);
            });
    }
}

impl Widget<egui::Ui> for BrowserView<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Ui as super::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: super::widget::Context<'a>,
    ) {
        self.draw(ui, ctx);
    }
}

/// Which address bar button was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrBarButton {
    Back,
    Forward,
    Refresh,
    UrlField,
}

impl BrowserView<'_> {
    /// Hit-test a click inside the address bar area.
    /// `lx` / `ly` are local coords relative to (bar_x, bar_y).
    pub fn address_bar_hit(
        lx: f32,
        ly: f32,
        bar_width: f32,
        styles: &Styles,
    ) -> Option<AddrBarButton> {
        let bar_h = styles.address_bar_height();
        let btn_w = styles.addr_btn_width();
        let btn_total = styles.addr_btn_total();
        if !(0.0..=bar_h).contains(&ly) || lx < 0.0 || lx > bar_width {
            return None;
        }
        if lx < btn_w {
            Some(AddrBarButton::Back)
        } else if lx < btn_w * 2.0 {
            Some(AddrBarButton::Forward)
        } else if lx < btn_total {
            Some(AddrBarButton::Refresh)
        } else {
            Some(AddrBarButton::UrlField)
        }
    }

    /// Hit-test a click in the browser view tab strip.
    /// Returns the index of the clicked tab, or None.
    pub fn browser_view_tab_hit(
        lx: f32,
        ly: f32,
        num_tabs: usize,
        styles: &Styles,
    ) -> Option<usize> {
        if !(0.0..=styles.browser_view_tab_width()).contains(&lx) || ly < 0.0 {
            return None;
        }
        let idx = (ly / styles.browser_view_tab_entry_height()) as usize;
        if idx < num_tabs { Some(idx) } else { None }
    }

    /// Hit-test a click in the terminal tab bar.
    /// Returns the session_id index of the clicked tab, or None.
    pub fn terminal_tab_hit(x: f32, _y: f32, num_tabs: usize, window_width: f32) -> Option<usize> {
        if num_tabs == 0 {
            return None;
        }
        let tab_w = Self::compute_tab_width(num_tabs, window_width);
        let idx = (x / tab_w) as usize;
        if idx < num_tabs { Some(idx) } else { None }
    }

    fn compute_tab_width(num_tabs: usize, window_width: f32) -> f32 {
        if num_tabs == 0 {
            return window_width;
        }
        let max_tab_width = 200.0_f32;
        let natural = window_width / num_tabs as f32;
        natural.min(max_tab_width)
    }
}

/// Address bar inline widget — wraps `draw_unified_address_bar`.
pub struct AddressBarWidget<'a> {
    pub url: &'a mut String,
    pub editing: &'a mut bool,
    pub close_webview_id: u32,
    /// When true, right-side action buttons are hidden (docked mode).
    pub docked: bool,
}

impl super::widget::Widget<egui::Ui> for AddressBarWidget<'_> {
    /// Unified address bar widget used by both overlay and docked webviews.
    /// Takes mutable references to url/editing state and an actions vec so it
    /// works from both UiState (overlay) and Behavior (docked tile).
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let styles = f.styles;
        let ui = &mut *f.painter;
        let tooltip = &mut f.dirties.float_tooltip;

        let mut actions = Vec::new();

        let close_webview_id = self.close_webview_id;
        let icon_size = styles.typography.title5;
        let small_sp = styles.spacing.small;

        let icon_btn = |ui: &mut egui::Ui, icon: Icon| -> egui::Response {
            icon_button(
                ui,
                IconButtonCfg {
                    icon,
                    tooltip: "",
                    base_color: ui.visuals().weak_text_color(),
                    hover_color: ui.visuals().text_color(),
                    pixel_size: icon_size,
                    margin: small_sp,
                },
            )
        };
        // Helper: check hover and build a TooltipRequest in main-window-logical coords.
        // Docked address bar tooltips are routed through the FloatPanel (a separate
        // NSPanel window) so they render above the native WKWebView/WebView2.
        let check_hover =
            |resp: &egui::Response,
             text: &str,
             tip: &mut Option<crate::ui::types::TooltipRequest>| {
                if resp.hovered() {
                    let center = resp.rect.center();
                    let bottom = resp.rect.max.y;
                    *tip = Some(crate::ui::types::TooltipRequest {
                        screen_x: center.x,
                        screen_y: bottom + 4.0,
                        text: text.to_string(),
                    });
                }
            };

        // Navigation buttons — pass "" to icon_button to suppress egui built-in tooltip.
        let back = icon_btn(ui, Icon::ArrowLeft);
        check_hover(&back, t("browser.back"), tooltip);
        if back.clicked() {
            actions.push(UiAction::WebviewBack(close_webview_id));
        }
        let fwd = icon_btn(ui, Icon::ArrowRight);
        check_hover(&fwd, t("browser.forward"), tooltip);
        if fwd.clicked() {
            actions.push(UiAction::WebviewForward(close_webview_id));
        }
        let refresh = icon_btn(ui, Icon::Refresh);
        check_hover(&refresh, t("browser.refresh"), tooltip);
        if refresh.clicked() {
            actions.push(UiAction::WebviewRefresh(close_webview_id));
        }

        // URL input.
        let right_buttons_width = if self.docked { 0.0 } else { 160.0 };
        let url_response = ui.add(
            egui::TextEdit::singleline(self.url)
                .desired_width(ui.available_width() - right_buttons_width)
                .font(egui::TextStyle::Small)
                .min_size(egui::vec2(0.0, 22.0))
                .vertical_align(egui::Align::Center),
        );
        if url_response.gained_focus() {
            *self.editing = true;
        }
        if url_response.lost_focus() {
            *self.editing = false;
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.push(UiAction::NavigateWebview(
                    self.url.clone(),
                    close_webview_id,
                ));
            }
        }

        // Right-side action buttons (split, open-as-tab, pop-out, open-system, close).
        // Hidden when docked — these actions are already in the tab bar.
        if !self.docked {
            ui.add_space(styles.spacing.medium);

            let split_h = icon_btn(ui, Icon::SplitHorizontal);
            check_hover(&split_h, t("browser.split_horizontal"), tooltip);
            if split_h.clicked() {
                actions.push(UiAction::DockWebviewRight);
            }
            let split_v = icon_btn(ui, Icon::SplitVertical);
            check_hover(&split_v, t("browser.split_vertical"), tooltip);
            if split_v.clicked() {
                actions.push(UiAction::DockWebviewDown);
            }
            let tab = icon_btn(ui, Icon::OpenInProduct);
            check_hover(&tab, t("browser.open_as_tab"), tooltip);
            if tab.clicked() {
                actions.push(UiAction::WebviewToTab(close_webview_id));
            }
            let ext = icon_btn(ui, Icon::Globe);
            check_hover(&ext, t("browser.open_system"), tooltip);
            if ext.clicked() {
                actions.push(UiAction::OpenInSystemBrowser);
            }
            let close = icon_btn(ui, Icon::Close);
            check_hover(&close, t("browser.close"), tooltip);
            if close.clicked() {
                actions.push(UiAction::CloseWebview(close_webview_id));
            }
        } // !docked

        f.dirties.actions.extend(actions);
    }
}

// ─── Browser Tab & Z-Order Management ────────────────────────────────────────

/// Unique browser identifier (same as `BrowserTab::browser.id`).
pub type BrowserId = u32;

/// A single browser tab entry.
pub struct BrowserTab {
    pub browser: Browser,
    /// Display mode: Overlay (floating) or Docked (split).
    pub mode: super::types::BrowserViewMode,
    /// Terminal session this overlay is paired with (if any).
    pub paired_session: Option<SessionId>,
    /// Docked webview ID this overlay is paired with (if opened from a webview tab).
    pub paired_webview: Option<BrowserId>,
    /// Optional overlay window for z-ordering + browser chrome rendering.
    /// On Linux, overlays fall back to the main window surface.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub overlay: Option<super::overlay::Modal>,
    /// Overlay's logical position (x, y) relative to the main window content area.
    /// Used to convert overlay-local tooltip coordinates to main-window coordinates.
    pub overlay_origin: (f32, f32),
}

/// The layer a browser occupies in the z-order stack.
///
/// Docked browsers are native child views of the main window and sit above
/// the wgpu surface (Metal / DX12 layer).  Overlay browsers live in
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

/// Per-browser metadata stored in the manager.
pub struct BrowserEntry {
    pub id: BrowserId,
    pub layer: ZLayer,
    /// For independent overlays: optional egui context for rendering
    /// UI browser within the child window.
    pub independent_egui: Option<IndependentEguiCtx>,
}

/// Egui context for an independent overlay window.
///
/// This allows rendering egui widgets (address bar, buttons) inside the
/// child window, on top of the browser.
#[allow(dead_code)]
#[derive(Default)]
pub struct IndependentEguiCtx {
    pub ctx: egui::Context,
    /// Address bar state for this independent overlay.
    pub address_bar_url: String,
    pub address_bar_editing: bool,
}

/// Lightweight z-order tracker for browser windows.
///
/// Works alongside the existing `browser_tabs: Vec<BrowserTab>` — does NOT
/// own `BrowserView` instances.  Register browser IDs after creating them,
/// and use the z-order API to manage overlay stacking.
#[derive(Default)]
pub struct BrowserManager {
    /// Per-browser metadata.
    entries: HashMap<BrowserId, BrowserEntry>,
    /// IDs of overlay/independent browsers sorted by activation order
    /// (most recently activated last = topmost).
    overlay_z_stack: Vec<BrowserId>,
}

impl BrowserManager {
    // ── Registration ────────────────────────────────────────────────────

    /// Register a webview ID with a given layer.
    pub fn register(&mut self, id: BrowserId, layer: ZLayer) {
        let entry = BrowserEntry {
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
    pub fn unregister(&mut self, id: BrowserId) {
        self.entries.remove(&id);
        self.overlay_z_stack.retain(|&wid| wid != id);
    }

    /// Get the layer for a webview.
    #[allow(dead_code)]
    pub fn layer(&self, id: BrowserId) -> Option<ZLayer> {
        self.entries.get(&id).map(|e| e.layer)
    }

    /// Get the entry for a webview.
    #[allow(dead_code)]
    pub fn get(&self, id: BrowserId) -> Option<&BrowserEntry> {
        self.entries.get(&id)
    }

    /// Get mutable entry.
    #[allow(dead_code)]
    pub fn get_mut(&mut self, id: BrowserId) -> Option<&mut BrowserEntry> {
        self.entries.get_mut(&id)
    }

    // ── Z-order management ──────────────────────────────────────────────

    /// Bring an overlay/independent browser to the top of the z-stack.
    pub fn bring_to_front(&mut self, id: BrowserId) -> &[BrowserId] {
        if let Some(entry) = self.entries.get(&id)
            && entry.layer == ZLayer::Docked
        {
            return &self.overlay_z_stack;
        }
        self.overlay_z_stack.retain(|&wid| wid != id);
        self.overlay_z_stack.push(id);
        &self.overlay_z_stack
    }

    /// Get the topmost overlay browser ID (if any are visible).
    #[allow(dead_code)]
    pub fn topmost_overlay(&self) -> Option<BrowserId> {
        self.overlay_z_stack.last().copied()
    }

    /// Get the overlay z-stack (bottom to top order).
    #[allow(dead_code)]
    pub fn overlay_stack(&self) -> &[BrowserId] {
        &self.overlay_z_stack
    }

    /// Remove a browser from the z-stack (e.g. when hiding it).
    #[allow(dead_code)]
    pub fn remove_from_z_stack(&mut self, id: BrowserId) {
        self.overlay_z_stack.retain(|&wid| wid != id);
    }

    /// Add to z-stack if not already present (e.g. when showing it).
    #[allow(dead_code)]
    pub fn add_to_z_stack(&mut self, id: BrowserId) {
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
    pub fn promote_to_overlay(&mut self, id: BrowserId) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.layer = ZLayer::Overlay;
        }
        if !self.overlay_z_stack.contains(&id) {
            self.overlay_z_stack.push(id);
        }
    }

    /// Demote an overlay browser to a docked (tile split) layer.
    #[allow(dead_code)]
    pub fn demote_to_docked(&mut self, id: BrowserId) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.layer = ZLayer::Docked;
            entry.independent_egui = None;
        }
        self.overlay_z_stack.retain(|&wid| wid != id);
    }

    /// Promote an overlay to an independent overlay with its own egui context.
    #[allow(dead_code)]
    pub fn promote_to_independent(&mut self, id: BrowserId) {
        self.promote_to_independent_with_url(id, "");
    }

    /// Promote to independent and initialize the address bar with a URL.
    pub fn promote_to_independent_with_url(&mut self, id: BrowserId, url: &str) {
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

    /// All docked browser IDs.
    #[allow(dead_code)]
    pub fn docked_ids(&self) -> Vec<BrowserId> {
        self.entries
            .values()
            .filter(|e| e.layer == ZLayer::Docked)
            .map(|e| e.id)
            .collect()
    }

    /// All overlay browser IDs (Overlay + Independent).
    #[allow(dead_code)]
    pub fn overlay_ids(&self) -> Vec<BrowserId> {
        self.entries
            .values()
            .filter(|e| e.layer == ZLayer::Overlay || e.layer == ZLayer::Independent)
            .map(|e| e.id)
            .collect()
    }

    /// All independent browser IDs.
    #[allow(dead_code)]
    pub fn independent_ids(&self) -> Vec<BrowserId> {
        self.entries
            .values()
            .filter(|e| e.layer == ZLayer::Independent)
            .map(|e| e.id)
            .collect()
    }

    /// Whether a browser is in the independent layer.
    #[allow(dead_code)]
    pub fn is_independent(&self, id: BrowserId) -> bool {
        self.entries
            .get(&id)
            .is_some_and(|e| e.layer == ZLayer::Independent)
    }

    /// Number of tracked browsers.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no tracked browsers.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
