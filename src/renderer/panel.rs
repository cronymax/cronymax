//! Cross-platform child window for hosting overlay webviews above docked views.
//!
//! | Platform    | Strategy                                               |
//! |-------------|--------------------------------------------------------|
//! | **macOS**   | NSPanel child window (floats above parent NSViews)     |
//! | **Windows** | Owned popup window via winit (floats above owner HWNDs) |
//! | **Linux**   | Not supported (fallback: hide docked + child-of-window) |
//!
//! Implements [`raw_window_handle::HasWindowHandle`] so wry can create a
//! WebView inside the panel via `WebViewBuilder::build_as_child`.

// ── FloatPanel ──────────────────────────────────────────────────────────────

// ── macOS NSEvent → egui::Event conversion ──────────────────────────────────
