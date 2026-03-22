//! Linux child window stub — not supported.

use std::sync::{Arc, Mutex};

use raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use super::{LogicalRect, PanelAttrs};

// ── Panel ──────────────────────────────────────────────────────────────

/// Stub child panel for platforms without native child-window support.
///
/// `new()` always returns `Err` — callers fall back to a child-of-window
/// webview or main-surface rendering.
#[allow(dead_code)]
pub struct Panel {
    pub visible: bool,
    pub event_buffer: Arc<Mutex<Vec<egui::Event>>>,
    pub last_cursor_pos: Arc<Mutex<egui::Pos2>>,
}

#[allow(dead_code)]
impl Panel {
    pub fn new(
        _parent: &Window,
        _event_loop: Option<&ActiveEventLoop>,
        _rect: LogicalRect,
        _attrs: PanelAttrs,
    ) -> Result<Self, String> {
        Err("Child panel windows not supported on this platform".into())
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_frame_logical(&self, _parent: &Window, _rect: super::LogicalRect) {}

    pub fn set_frame(&self, _parent: &Window, _sx: f32, _sy: f32, _w: f32, _h: f32, _scale: f32) {}

    pub fn ensure_above_overlays(&self) {}

    pub fn install_event_monitor(&mut self, _panel_logical_height: f32) {}

    pub fn window_id(&self) -> Option<winit::window::WindowId> {
        None
    }
}

impl HasWindowHandle for Panel {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}
