//! UI widget system — implements the five-layer widget hierarchy.
//!
//! See [`types`] module for the full hierarchy documentation and
//! [`PaneKind`], [`OverlayKind`], [`FloatKind`] enum definitions.
//!
//! # Module mapping
//!
//! | Layer        | Module       | Description                                  |
//! |--------------|--------------|----------------------------------------------|
//! | §1 Titlebar    | [`titlebar`] | macOS controls, pinned tabs, actions         |
//! | §2 Tiles       | [`tiles`]    | tabs bar, pane tree (egui_tiles)             |
//! | §2.3 Block     | [`block`]    | terminal (PTY) / stream (SSE) blocks         |
//! | §2.4 Prompt    | [`prompt`]   | suggestion panel, prompt editor              |
//! | §3 Overlay     | [`settings`] | settings overlay (§3.2)                      |
//! | §3 Overlay     | [`browser`]  | browser view overlay (§3.1)                  |
//! | §4 Float       |  —           | tooltips / dialogs (webview/child_panel)     |
//! | §5 BrowserView | [`browser`]  | address bar + webview chrome                 |
//! | Support      | [`styles`]   | theme, spacing, typography                   |
//! | Support      | [`icons`]    | SVG icon system                              |
//! | Support      | [`i18n`]     | internationalization                         |
//! | Support      | [`chat`]     | per-session LLM chat state                   |
//! | Support      | [`completion`] | path auto-completion                       |
//! | Support      | [`file_picker`] | `#`-triggered inline file picker          |
//! | Legacy       | [`overlay`]  | quad-based rendering (hit testing fallback)  |

pub mod actions;
pub mod block;
pub mod browser;
pub mod chat;
pub mod completion;
pub mod file_picker;
mod filter;
pub mod frame;
pub mod i18n;
pub mod icons;
pub mod prompt;
mod relaunch_dialog;
pub mod settings;
pub mod skills_panel;
pub mod styles;
pub mod tiles;
mod titlebar;
pub mod types;
pub mod widget;

// Re-export all public types from types.rs and actions.rs at the `ui::` level.
pub use actions::*;
pub use types::*;

// /// Something to view in the demo windows
// pub trait View {
//     fn ui(&mut self, ui: &mut egui::Ui);
// }

/// Convert [f32; 4] RGBA to egui Color32. Used by UI sub-modules.
pub(crate) use crate::renderer::atlas::CellSize;
use crate::ui::{styles::Styles, widget::Fragment};

/// A rectangular region in the window (in physical pixels).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Default padding around the terminal content in pixels.
const PADDING: f32 = 4.0;

impl Viewport {
    /// Compute the terminal viewport from window dimensions, applying padding.
    #[allow(dead_code)]
    pub fn from_window_size(window_width: u32, window_height: u32) -> Self {
        let w = window_width as f32;
        let h = window_height as f32;
        Self {
            x: PADDING,
            y: PADDING,
            width: (w - 2.0 * PADDING).max(0.0),
            height: (h - 2.0 * PADDING).max(0.0),
        }
    }

    /// Compute viewport with the top offset for the tab bar.
    pub fn from_window_with_tab_bar(
        window_width: u32,
        window_height: u32,
        styles: &Styles,
    ) -> Self {
        let w = window_width as f32;
        let h = window_height as f32;
        let top = styles.tab_bar_height() + PADDING;
        Self {
            x: PADDING,
            y: top,
            width: (w - 2.0 * PADDING).max(0.0),
            height: (h - top - PADDING).max(0.0),
        }
    }

    /// Calculate grid dimensions (columns, rows) from this viewport and cell size.
    pub fn grid_dimensions(&self, cell: &CellSize) -> (u16, u16) {
        let cols = (self.width / cell.width).floor().max(1.0) as u16;
        let rows = (self.height / cell.height).floor().max(1.0) as u16;
        (cols, rows)
    }
}

/// Compute the viewport and grid size for a single-pane layout (with tab bar).
pub fn compute_single_pane(
    window_width: u32,
    window_height: u32,
    cell: &CellSize,
    styles: &Styles,
) -> (Viewport, u16, u16) {
    let viewport = Viewport::from_window_with_tab_bar(window_width, window_height, styles);
    let (cols, rows) = viewport.grid_dimensions(cell);
    (viewport, cols, rows)
}

/// Draw all egui UI widgets. Returns the list of actions for the app to process,
/// per-pane submitted commands, and the pane rects collected from egui_tiles layout.
///
/// Uses the [`widget::PanelWidget`] trait with [`widget::WidgetResponse::merge()`]
/// to compose top-level panels in rendering order.
#[allow(clippy::type_complexity)]
pub fn draw_all(tiles: tiles::TilesPanel<'_>, f: &mut Fragment<'_, egui::Context>) {
    let ctx = f.ctx();
    // ── Window frame: rounded corners ─────────────────────────────────
    ctx.layer_painter(egui::LayerId::background()).rect_filled(
        ctx.screen_rect(),
        egui::CornerRadius::from(f.styles.spacing.large),
        f.colors.bg_body,
    );

    // child widgets
    {
        // §1  Titlebar
        f.add(titlebar::TitlebarWidget);

        // §1.5  Filter bar
        f.add(filter::FilterBarWidget);

        // §3.1  Browser overlay (before CentralPanel so it claims screen space first)
        f.add(browser::BrowserOverlay);

        // §2  Tiles (CentralPanel — must be last panel claim)
        f.add(tiles);
    }

    // ── Window border stroke (on top of everything) ─────────────────────
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window_border"),
    ))
    .rect_stroke(
        ctx.screen_rect(),
        egui::CornerRadius::from(f.styles.spacing.large),
        egui::Stroke::new(f.styles.sizes.border, f.colors.border),
        egui::StrokeKind::Inside,
    );

    // The docked tooltip is still communicated via the tile behavior's field.
    // TilesPanel doesn't surface it through WidgetResponse (it's a separate concern
    // routed to FloatPanel), so we extract it from the tiles behavior state that
    // was stored in ui_state during TilesPanel::show().
    f.dirties.mount_tooltip(f.ui_state.docked_tooltip.take());
}
