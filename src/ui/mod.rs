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
pub mod blocks;
pub mod browser;
pub mod chat;
pub mod command_palette;
pub mod completion;
pub mod dispatch;
pub(crate) mod draw;
pub mod file_picker;
mod filter;
pub mod frame;
pub mod i18n;
pub mod icons;
pub mod keybindings;
pub mod model;
pub mod mouse;
pub mod notifications;
pub mod overlay;
pub mod prompt;
mod relaunch_dialog;
pub mod settings;
pub mod skills_panel;
pub mod styles;
pub mod sync;
pub mod tiles;
mod titlebar;
pub mod types;
pub mod view;
pub mod widget;

use std::sync::Arc;

// Re-export all public types from types.rs and actions.rs at the `ui::` level.
pub use actions::*;
pub use types::*;

pub use view::{View, ViewMut};

pub use crate::renderer::viewport::Viewport;
use crate::ui::styles::Styles;

/// Convert [f32; 4] RGBA to egui Color32. Used by UI sub-modules.
pub(crate) use crate::renderer::atlas::CellSize;

use winit::keyboard::ModifiersState;
use winit::window::Window;

use crate::renderer::frame::Frame;
use crate::ui::browser::{BrowserId, BrowserManager, BrowserTab};
use crate::ui::overlay::float::FloatPanelState;
pub use crate::ui::types::UiState;

// ─── The model ───────────────────────────────────────────────────────────────

/// Central UI model — groups everything that lives on the "view" side of the
/// architecture.  Held as `AppState.ui`.
pub(crate) struct Ui {
    // ── Rendering ────────────────────────────────────────────────────────
    /// Main window + GPU context + egui integration.
    pub(crate) frame: Frame,
    /// Theme (colors + spacing).
    pub(crate) styles: Styles,
    /// Terminal viewport (physical-pixel rectangle).
    pub(crate) viewport: Viewport,

    // ── Layout ───────────────────────────────────────────────────────────
    /// Tiling layout tree (egui_tiles).
    pub(crate) tile_tree: egui_tiles::Tree<tiles::Pane>,
    /// Tile rects collected each frame for wgpu viewport mapping & browser positioning.
    pub(crate) tile_rects: Vec<tiles::TileRect>,

    // ── Browser / overlay ────────────────────────────────────────────────
    /// All open browser tabs.
    pub(crate) browser_tabs: Vec<BrowserTab>,
    /// Active browser tab index.
    pub(crate) active_browser: usize,
    /// Next browser ID counter.
    pub(crate) next_browser_id: BrowserId,
    /// Z-order tracking for browser overlays.
    pub(crate) browser_manager: BrowserManager,
    /// Per-frame transient state for the Float Panel tooltip rendering.
    pub(crate) float_panel_state: FloatPanelState,
    /// Float renderer (tier 3) — tooltip window above all overlays.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) float_renderer: Option<crate::ui::overlay::Float>,
    /// Overlay renderer (tier 2) for the Settings page.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) settings_overlay: Option<crate::ui::overlay::Modal>,
    /// Split layout when browser is docked beside the terminal.
    pub(crate) split: Option<VerticalSplit>,

    // ── Input ────────────────────────────────────────────────────────────
    /// Current mouse position in physical pixels.
    pub(crate) mouse_x: f32,
    pub(crate) mouse_y: f32,
    /// Currently hovered link (when Cmd/Ctrl is held and mouse over a link).
    pub(crate) hovered_link: Option<crate::renderer::terminal::links::DetectedLink>,
    /// Current keyboard modifier state.
    pub(crate) modifiers: ModifiersState,
    /// Whether an IME composition (preedit) is currently active.
    pub(crate) ime_composing: bool,
    /// Whether the IME input method is enabled.
    pub(crate) ime_enabled: bool,
    /// Accumulated input buffer for `:command` mode.
    pub(crate) colon_buf: Option<String>,
    /// Whether Tab was pressed this frame (detected in `prepare_raw_input`,
    /// consumed by prompt widgets via egui temp data).
    pub(crate) tab_pressed: bool,
    /// URL intercepted from egui hyperlink clicks during `manipulate_full_output`.
    /// Drained by `draw_frame` to open in the overlay browser.
    pub(crate) intercepted_url: Option<String>,

    // ── Terminal text selection ───────────────────────────────────────
    /// Active text selection in the terminal (mouse drag).
    pub(crate) terminal_selection: Option<TerminalSelection>,
    /// Whether a mouse drag is in progress for text selection.
    pub(crate) selection_dragging: bool,
}

/// Terminal text selection state — tracks a rectangular region across terminal rows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TerminalSelection {
    /// Session ID this selection belongs to.
    pub session_id: crate::renderer::terminal::SessionId,
    /// Start column (0-indexed, cell coordinates).
    pub start_col: usize,
    /// Start row (viewport-relative, 0-indexed).
    pub start_row: usize,
    /// End column (0-indexed, cell coordinates).
    pub end_col: usize,
    /// End row (viewport-relative, 0-indexed).
    pub end_row: usize,
}

impl TerminalSelection {
    /// Return the selection range normalized so start <= end.
    pub fn normalized(&self) -> (usize, usize, usize, usize) {
        if self.start_row < self.end_row
            || (self.start_row == self.end_row && self.start_col <= self.end_col)
        {
            (self.start_col, self.start_row, self.end_col, self.end_row)
        } else {
            (self.end_col, self.end_row, self.start_col, self.start_row)
        }
    }

    /// Check if a cell (col, row) is within this selection.
    #[allow(dead_code)]
    pub fn contains(&self, col: usize, row: usize) -> bool {
        let (sc, sr, ec, er) = self.normalized();
        if row < sr || row > er {
            return false;
        }
        if sr == er {
            // Single-line selection.
            col >= sc && col <= ec
        } else if row == sr {
            col >= sc
        } else if row == er {
            col <= ec
        } else {
            true // middle rows are fully selected
        }
    }
}

// ─── Convenience API for app/ ────────────────────────────────────────────────

#[allow(dead_code)]
impl Ui {
    // ── Window accessors ─────────────────────────────────────────────────

    /// Reference to the underlying winit `Window`.
    #[inline]
    pub(crate) fn window(&self) -> &Arc<Window> {
        &self.frame.window
    }

    /// DPI scale factor.
    #[inline]
    pub(crate) fn scale(&self) -> f32 {
        self.frame.scale()
    }

    /// Physical inner size.
    #[inline]
    pub(crate) fn inner_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.frame.window.inner_size()
    }

    // ── Event routing ────────────────────────────────────────────────────

    /// Forward a winit `WindowEvent` to egui (returns `true` if consumed).
    #[inline]
    pub(crate) fn on_window_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        self.frame.on_window_event(event)
    }

    /// Resize the GPU surface after a window resize.
    #[inline]
    pub(crate) fn resize_surface(&mut self, width: u32, height: u32) {
        self.frame.gpu.resize(width, height);
    }

    // ── Input ────────────────────────────────────────────────────────────

    /// Map a key event through the current modifiers to an action.
    #[inline]
    pub(crate) fn match_keybinding(&self, event: &winit::event::KeyEvent) -> Option<KeyAction> {
        keybindings::match_keybinding(event, &self.modifiers)
    }
}

impl View for Ui {
    type Renderer = Frame;
    fn as_renderer(&self) -> &Self::Renderer {
        &self.frame
    }
}
impl ViewMut for Ui {
    fn as_mut_renderer(&mut self) -> &mut Self::Renderer {
        &mut self.frame
    }

    fn prepare_raw_input(&mut self) -> egui::RawInput {
        let mut raw_input = self.frame.take_egui_input();

        // Strip Tab key events from raw input BEFORE ctx.run().
        // egui processes Tab for focus-cycling in begin_pass() which runs before
        // any widget code.  Removing Tab here prevents focus from moving away
        // from the prompt TextEdit so path-completion / suggestion-select works.
        self.tab_pressed = raw_input.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    ..
                }
            )
        });
        if self.tab_pressed {
            raw_input.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Tab,
                        ..
                    }
                )
            });
        }

        // Propagate to egui temp data so widgets can detect tab presses
        // during the upcoming run_ui pass.
        self.frame.egui.ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("__global_tab_pressed"), self.tab_pressed);
        });

        raw_input
    }

    fn manipulate_full_output(&mut self, mut full_output: egui::FullOutput) -> egui::FullOutput {
        // Check the new `commands` vector first (egui 0.31+ uses OutputCommand::OpenUrl).
        let mut open_url: Option<String> = None;
        full_output.platform_output.commands.retain(|cmd| {
            if let egui::OutputCommand::OpenUrl(ou) = cmd {
                open_url = Some(ou.url.clone());
                false // remove so egui-winit doesn't also open system browser
            } else {
                true
            }
        });
        // Fallback: also check the deprecated field.
        if open_url.is_none() {
            #[allow(deprecated)]
            if let Some(ou) = full_output.platform_output.open_url.take() {
                open_url = Some(ou.url);
            }
        }

        // Redirect egui hyperlink clicks (from markdown rendering) to
        // the built-in overlay browser instead of the system browser.
        if let Some(url) = open_url {
            log::info!("Intercepted egui link click → opening in overlay: {}", url);
            self.intercepted_url = Some(url);
        }

        full_output
    }

    fn handle_platform_output(&mut self, platform_output: egui::PlatformOutput) {
        self.frame.handle_platform_output(platform_output);
    }
}

/// Default padding around the terminal content in pixels.
const PADDING: f32 = 4.0;

/// Compute the viewport and grid size for a single-pane layout (with tab bar).
pub fn compute_single_pane(
    window_width: u32,
    window_height: u32,
    cell: &CellSize,
    styles: &Styles,
) -> (Viewport, u16, u16) {
    let w = window_width as f32;
    let h = window_height as f32;
    let top = styles.tab_bar_height() + PADDING;

    let viewport = Viewport {
        x: PADDING,
        y: top,
        width: (w - 2.0 * PADDING).max(0.0),
        height: (h - top - PADDING).max(0.0),
    };
    let (cols, rows) = viewport.grid_dimensions(cell);
    (viewport, cols, rows)
}

/// A vertical split: terminal on the left, webview on the right.
#[derive(Debug, Clone, Copy)]
pub struct VerticalSplit {
    /// Fraction of window width allocated to the terminal (0.0–1.0).
    pub ratio: f32,
}

impl Default for VerticalSplit {
    fn default() -> Self {
        Self { ratio: 0.5 }
    }
}

impl VerticalSplit {
    /// Compute the terminal viewport (left side) given window dimensions.
    /// Accounts for tab bar height at top.
    pub fn terminal_viewport(
        &self,
        window_width: u32,
        window_height: u32,
        styles: &Styles,
    ) -> Viewport {
        let padding = 4.0;
        let top = styles.tab_bar_height() + padding;
        let term_width = (window_width as f32 * self.ratio) - 2.0 * padding;
        Viewport {
            x: padding,
            y: top,
            width: term_width.max(0.0),
            height: (window_height as f32 - top - padding).max(0.0),
        }
    }

    /// Compute the webview bounds (right side) given window dimensions.
    /// Accounts for tab bar height, webview tab strip, and address bar.
    pub fn webview_bounds(
        &self,
        window_width: u32,
        window_height: u32,
        styles: &Styles,
    ) -> Viewport {
        let left_width = (window_width as f32 * self.ratio) as u32;
        let top = styles.tab_bar_height() as u32 + styles.address_bar_height() as u32;
        let wv_tab_strip = styles.browser_view_tab_width() as u32;
        Viewport::new(
            (left_width + wv_tab_strip) as _,
            top as _,
            window_width
                .saturating_sub(left_width)
                .saturating_sub(wv_tab_strip) as _,
            window_height.saturating_sub(top) as _,
        )
    }

    /// Compute the area where the browser view tab strip is drawn.
    pub fn browser_view_tab_area(
        &self,
        window_width: u32,
        window_height: u32,
        styles: &Styles,
    ) -> (f32, f32, f32) {
        let left_width = window_width as f32 * self.ratio;
        let top = styles.tab_bar_height() + styles.address_bar_height();
        (left_width, top, window_height as f32 - top)
    }

    /// Compute the address bar area (above webview).
    pub fn address_bar_area(&self, window_width: u32, styles: &Styles) -> (f32, f32, f32) {
        let left_width = window_width as f32 * self.ratio;
        let top = styles.tab_bar_height();
        let bar_width = window_width as f32 - left_width;
        (left_width, top, bar_width)
    }

    /// Compute grid dimensions for the terminal side of the split.
    pub fn terminal_grid(
        &self,
        window_width: u32,
        window_height: u32,
        cell: &CellSize,
        styles: &Styles,
    ) -> (Viewport, u16, u16) {
        let vp = self.terminal_viewport(window_width, window_height, styles);
        let (cols, rows) = vp.grid_dimensions(cell);
        (vp, cols, rows)
    }
}
