//! Cross-platform child window for hosting overlays above base views.
//!
//! | Platform    | Strategy                                               |
//! |-------------|--------------------------------------------------------|
//! | **macOS**   | NSPanel child window (floats above parent NSViews)     |
//! | **Windows** | Owned popup window via winit (floats above owner HWNDs) |
//! | **Linux**   | Not supported (fallback: hide docked + child-of-window) |
//!
//! Each platform provides a [`ChildPanel`] type with the same public API.
//! Higher-level widgets [`Modal`](crate::ui::overlay::Modal) and
//! [`Float`](crate::ui::overlay::Float) configure the panel appropriately
//! — callers should use those rather than constructing `ChildPanel` directly.
//!
//! ## Z-index hierarchy
//!
//! ```text
//! +-----------------------------------------------------+
//! |  Float  (tooltips, popovers)                 |  z = 3
//! +-----------------------------------------------------+
//! |  Modal  (browser overlays, settings)         |  z = 2
//! +-----------------------------------------------------+
//! |  BaseRenderer  (main window, terminals, egui)|  z = 1
//! +-----------------------------------------------------+
//! ```

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use linux::ChildPanel;
#[cfg(target_os = "macos")]
pub use macos::Panel;
#[cfg(target_os = "windows")]
pub use windows::Panel;

/// A rectangle in logical coordinates with a DPI scale factor.
///
/// Groups the `(x, y, w, h, scale)` quintuple that appears throughout the
/// overlay/panel APIs, reducing function argument count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalRect {
    /// Logical x position relative to parent content area.
    pub x: f32,
    /// Logical y position relative to parent content area.
    pub y: f32,
    /// Logical width.
    pub w: f32,
    /// Logical height.
    pub h: f32,
    /// DPI scale factor.
    pub scale: f32,
}

/// Platform-agnostic configuration for creating a child window.
///
/// Higher-level widgets `Modal` and `Float` set these fields
/// appropriately — callers should rarely construct this directly.
#[derive(Debug, Clone)]
pub struct PanelAttrs {
    /// Draw a drop shadow around the panel.
    pub shadow: bool,
    /// Panel can become the key window (receive keyboard focus).
    pub focusable: bool,
    /// Mouse events pass through to windows below.
    pub click_through: bool,
    /// Window level offset from parent (0 = same, 1 = one above, etc.).
    pub level_offset: i64,
    /// Whether the panel starts visible.
    pub initially_visible: bool,
    /// Corner radius for the content view layer (0.0 = no rounding).
    pub corner_radius: f64,
    /// Make the panel fully opaque (no compositor alpha blending).
    /// The CALayer corner mask still clips visible content.
    pub opaque: bool,
}
