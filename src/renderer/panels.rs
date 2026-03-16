//! Cross-platform child window for hosting overlay above base views.
//!
//! | Platform    | Strategy                                               |
//! |-------------|--------------------------------------------------------|
//! | **macOS**   | NSPanel child window (floats above parent NSViews)     |
//! | **Windows** | Owned popup window via winit (floats above owner HWNDs) |
//! | **Linux**   | Not supported (fallback: hide docked + child-of-window) |
//!
//! Implements [`raw_window_handle::HasWindowHandle`] so wry can create a
//! WebView inside the panel via `WebViewBuilder::build_as_child`.
//!
//! ## Z-index hierarchy
//!
//! ```text
//! +-----------------------------------------------------+
//! |  Float  (tooltips, popovers)                 |  z = 3
//! +-----------------------------------------------------+
//! |  Modal  (browser overlays, settings)       |  z = 2
//! +-----------------------------------------------------+
//! |  BaseRenderer  (main window, terminals, egui)        |  z = 1
//! +-----------------------------------------------------+
//! ```
//!
//! Each tier wraps a platform window + GPU surface + egui context:
//!
//! - **[`Modal`]**: A child window (NSPanel / owned popup) that floats
//!   above native webviews.
//! - **[`Float`]**: A non-interactive child window for tooltips that floats
//!   above everything. Click-through, ignores mouse events, highest z-order.
mod float;
mod modal;

pub use float::{FloatPanel, FloatPanelState};
pub use modal::ModalPanel;
