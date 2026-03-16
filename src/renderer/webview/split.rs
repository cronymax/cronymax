//! Webview panel positioning and resize within window.

use crate::renderer::atlas::CellSize;
use crate::ui::Viewport;
use crate::ui::styles::Styles;
use wry::Rect;
use wry::dpi::{PhysicalPosition, PhysicalSize};

/// Describes the position and size of a webview panel in physical pixels.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Bounds {
    /// Create panel bounds from physical pixel values.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Convert to a wry Rect for set_bounds().
    pub fn to_wry_rect(self) -> Rect {
        Rect {
            position: PhysicalPosition::new(self.x, self.y).into(),
            size: PhysicalSize::new(self.width, self.height).into(),
        }
    }
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
    pub fn webview_bounds(&self, window_width: u32, window_height: u32, styles: &Styles) -> Bounds {
        let left_width = (window_width as f32 * self.ratio) as u32;
        let top = styles.tab_bar_height() as u32 + styles.address_bar_height() as u32;
        let wv_tab_strip = styles.browser_view_tab_width() as u32;
        Bounds::new(
            left_width + wv_tab_strip,
            top,
            window_width
                .saturating_sub(left_width)
                .saturating_sub(wv_tab_strip),
            window_height.saturating_sub(top),
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
