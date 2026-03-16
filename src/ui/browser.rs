//! Browser view widget (§5) and browser view overlay (§3.1).
//!
//! - [`BrowserOverlay`]: overlay sizing/positioning + hit-testing + user agent
//! - [`AddressBarWidget`]: shared address bar for docked & overlay (§5)

use crate::ui::i18n::t;

use super::actions::UiAction;
use super::icons::{self, Icon};
use super::styles::Styles;
use super::types::BrowserViewMode;
use super::widget::{Fragment, Widget};

// ─── Browser Icon Buttons (SVG icon-based) ─────────────────────────────────

/// Icon types for browser address bar buttons.
pub enum BrowserIcon {
    Back,
    Forward,
    Refresh,
    SplitHorizontal,
    SplitVertical,
    OpenAsTab,
    #[allow(dead_code)]
    PopOut,
    ExternalLink,
    Close,
}

impl BrowserIcon {
    /// Map to the corresponding SVG icon.
    fn to_svg_icon(&self) -> Icon {
        match self {
            BrowserIcon::Back => Icon::ArrowLeft,
            BrowserIcon::Forward => Icon::ArrowRight,
            BrowserIcon::Refresh => Icon::Refresh,
            BrowserIcon::SplitHorizontal => Icon::SplitHorizontal,
            BrowserIcon::SplitVertical => Icon::SplitVertical,
            BrowserIcon::OpenAsTab => Icon::OpenInProduct,
            BrowserIcon::PopOut => Icon::ChromeMaximize,
            BrowserIcon::ExternalLink => Icon::Globe,
            BrowserIcon::Close => Icon::Close,
        }
    }
}

// ─── Webview overlay ────────────────────────────────────────────────────────

/// Browser overlay panel widget — wraps `draw_webview_overlay`.
pub struct BrowserOverlay;

impl Widget for BrowserOverlay {
    /// Draw the floating webview overlay frame (border + shadow only).
    ///
    /// The address bar browser is rendered exclusively by the overlay child window's
    /// `render_browser()` in child_gpu.rs (three-layer architecture).
    /// This function only renders the decorative border/shadow backdrop on the main
    /// window egui surface, and computes positioning rects for the child panel.
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        let ctx = f.ctx();
        let (styles, ui_state) = (f.styles, &mut *f.ui_state);
        let is_overlay = ui_state
            .active_webview_id
            .and_then(|wid| {
                ui_state
                    .tabs
                    .iter()
                    .find(|t| t.is_webview() && t.id() == wid)
            })
            .is_some_and(|t| {
                matches!(
                    t,
                    crate::ui::types::TabInfo::BrowserView {
                        mode: BrowserViewMode::Overlay,
                        ..
                    }
                )
            });

        if !is_overlay {
            ui_state.overlay_content_rect = None;
            ui_state.overlay_panel_rect = None;
            return;
        }

        let screen = ctx.screen_rect();
        let pop_w = (screen.width() * 0.80)
            .min(screen.width() - 40.0)
            .max(300.0);
        let pop_h = (screen.height() * 0.70)
            .min(screen.height() - 80.0)
            .max(200.0);
        let address_bar_h = styles.address_bar_height();
        let bw = styles.sizes.border;

        // Pure sizing/positioning calculator — NO visual rendering.
        // Border stroke is rendered by the child window's render_browser();
        // shadow is provided by the native platform (NSPanel.setHasShadow(true)).
        // We use an invisible egui::Area only to compute centered rects.
        egui::Area::new(egui::Id::new("webview_overlay_area"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                // Invisible frame — no fill, no stroke, no shadow.
                let _frame_resp = egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::NONE)
                    .inner_margin(egui::Margin::same(0))
                    .show(ui, |ui| {
                        ui.set_width(pop_w);
                        ui.set_height(pop_h);
                    });

                // Derive overlay rects from the invisible frame's position.
                let wr = _frame_resp.response.rect;

                // Full overlay panel rect (child window occupies this space).
                ui_state.overlay_panel_rect = Some([wr.min.x, wr.min.y, wr.width(), wr.height()]);

                // Content rect (webview area, below address bar, inset from borders).
                // The y-offset still accounts for address_bar_h because the child
                // window's browser occupies that space at the top of the panel.
                ui_state.overlay_content_rect = Some([
                    wr.min.x + bw,
                    wr.min.y + bw + address_bar_h,
                    pop_w,
                    pop_h - address_bar_h,
                ]);
            });
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

impl BrowserOverlay {
    /// OS-specific browser User-Agent string.
    /// Returns a Safari UA on macOS, Edge UA on Windows, Firefox UA on Linux.
    /// Pass this to `wry::WebViewBuilder::with_user_agent()` when constructing the
    /// native webview so sites receive a standard desktop-browser fingerprint.
    pub fn browser_user_agent() -> &'static str {
        #[cfg(target_os = "macos")]
        {
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) \
             AppleWebKit/605.1.15 (KHTML, like Gecko) \
             Version/17.5 Safari/605.1.15"
        }
        #[cfg(target_os = "windows")]
        {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/131.0.0.0 Safari/537.36 \
             Edg/131.0.0.0"
        }
        #[cfg(target_os = "linux")]
        {
            "Mozilla/5.0 (X11; Linux x86_64; rv:133.0) \
             Gecko/20100101 Firefox/133.0"
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            "Mozilla/5.0 (Unknown; rv:133.0) Gecko/20100101 Firefox/133.0"
        }
    }

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
    pub tooltip: &'a mut Option<crate::ui::types::TooltipRequest>,
}

impl AddressBarWidget<'_> {
    /// Draw a custom browser icon button using the Painter API.
    fn browser_icon_button(
        ui: &mut egui::Ui,
        icon: BrowserIcon,
        hover_text: &str,
        styles: &Styles,
    ) -> egui::Response {
        icons::icon_button(
            ui,
            icons::IconButtonCfg {
                icon: icon.to_svg_icon(),
                tooltip: hover_text,
                base_color: ui.visuals().weak_text_color(),
                hover_color: ui.visuals().text_color(),
                pixel_size: styles.typography.title5,
                margin: styles.spacing.small,
            },
        )
    }
}

impl super::widget::Widget<egui::Ui> for AddressBarWidget<'_> {
    /// Unified address bar widget used by both overlay and docked webviews.
    /// Takes mutable references to url/editing state and an actions vec so it
    /// works from both UiState (overlay) and Behavior (docked tile).
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let styles = f.styles;
        let Self { tooltip, .. } = self;
        let ui = f.ui();

        let mut actions = Vec::new();

        let close_webview_id = self.close_webview_id;

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
        let back = Self::browser_icon_button(ui, BrowserIcon::Back, "", styles);
        check_hover(&back, t("browser.back"), tooltip);
        if back.clicked() {
            actions.push(UiAction::WebviewBack(close_webview_id));
        }
        let fwd = Self::browser_icon_button(ui, BrowserIcon::Forward, "", styles);
        check_hover(&fwd, t("browser.forward"), tooltip);
        if fwd.clicked() {
            actions.push(UiAction::WebviewForward(close_webview_id));
        }
        let refresh = Self::browser_icon_button(ui, BrowserIcon::Refresh, "", styles);
        check_hover(&refresh, t("browser.refresh"), tooltip);
        if refresh.clicked() {
            actions.push(UiAction::WebviewRefresh(close_webview_id));
        }

        // URL input.
        let url_response = ui.add(
            egui::TextEdit::singleline(self.url)
                .desired_width(ui.available_width() - 200.0)
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
        ui.add_space(styles.spacing.medium);

        let split_h = Self::browser_icon_button(ui, BrowserIcon::SplitHorizontal, "", styles);
        check_hover(&split_h, t("browser.split_horizontal"), tooltip);
        if split_h.clicked() {
            actions.push(UiAction::DockWebviewRight);
        }
        let split_v = Self::browser_icon_button(ui, BrowserIcon::SplitVertical, "", styles);
        check_hover(&split_v, t("browser.split_vertical"), tooltip);
        if split_v.clicked() {
            actions.push(UiAction::DockWebviewDown);
        }
        let tab = Self::browser_icon_button(ui, BrowserIcon::OpenAsTab, "", styles);
        check_hover(&tab, t("browser.open_as_tab"), tooltip);
        if tab.clicked() {
            actions.push(UiAction::WebviewToTab(close_webview_id));
        }
        let ext = Self::browser_icon_button(ui, BrowserIcon::ExternalLink, "", styles);
        check_hover(&ext, t("browser.open_system"), tooltip);
        if ext.clicked() {
            actions.push(UiAction::OpenInSystemBrowser);
        }
        let close = Self::browser_icon_button(ui, BrowserIcon::Close, "", styles);
        check_hover(&close, t("browser.close"), tooltip);
        if close.clicked() {
            actions.push(UiAction::CloseWebview(close_webview_id));
        }

        f.dirties.actions.extend(actions);
    }
}
