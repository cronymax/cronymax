//! Windows child window implementation using winit owned popup.

use std::sync::{Arc, Mutex};

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use super::{LogicalRect, PanelAttrs};

/// Platform child window backed by a winit owned popup.
///
/// Configuration (click-through, etc.) is driven entirely by
/// [`PanelAttrs`] — the panel itself is style-agnostic.
#[allow(dead_code)]
pub struct Panel {
    window: Window,
    pub visible: bool,
    /// Shared event buffer for OS-level event → egui::Event forwarding.
    pub event_buffer: Arc<Mutex<Vec<egui::Event>>>,
    /// Last known cursor position in logical coordinates.
    pub last_cursor_pos: Arc<Mutex<egui::Pos2>>,
}

#[allow(dead_code)]
impl Panel {
    /// Create a new child panel with the given configuration.
    pub fn new(
        parent: &Window,
        event_loop: Option<&ActiveEventLoop>,
        rect: LogicalRect,
        attrs: PanelAttrs,
    ) -> Result<Self, String> {
        use winit::platform::windows::WindowAttributesExtWindows;

        let event_loop =
            event_loop.ok_or_else(|| "ActiveEventLoop required on Windows".to_string())?;

        let handle = parent.window_handle().map_err(|e| format!("{e}"))?;
        let hwnd = match handle.as_raw() {
            RawWindowHandle::Win32(h) => h.hwnd.get(),
            _ => return Err("Expected Win32 window handle".into()),
        };

        let mut attrs = Window::default_attributes()
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_visible(false)
            .with_owner_window(hwnd)
            .with_skip_taskbar(true);

        if rect.w > 0.0 && rect.h > 0.0 {
            let inner_pos = parent.inner_position().map_err(|e| format!("{e}"))?;
            let sx = inner_pos.x + (rect.x * rect.scale) as i32;
            let sy = inner_pos.y + (rect.y * rect.scale) as i32;
            let pw = (rect.w * rect.scale) as u32;
            let ph = (rect.h * rect.scale) as u32;
            attrs = attrs
                .with_inner_size(PhysicalSize::new(pw, ph))
                .with_position(PhysicalPosition::new(sx, sy));
        } else {
            attrs = attrs.with_inner_size(PhysicalSize::new(1u32, 1u32));
        }

        let window = event_loop
            .create_window(attrs)
            .map_err(|e| format!("Failed to create child panel window: {e}"))?;

        // Apply WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW
        // for click-through panels.
        if attrs.click_through {
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::Win32(h) = handle.as_raw()
            {
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::*;
                    let hwnd_val = h.hwnd.get() as *mut core::ffi::c_void;
                    let ex_style = GetWindowLongW(hwnd_val, GWL_EXSTYLE);
                    let new_style = ex_style
                        | WS_EX_NOACTIVATE as i32
                        | WS_EX_TRANSPARENT as i32
                        | WS_EX_TOOLWINDOW as i32;
                    SetWindowLongW(hwnd_val, GWL_EXSTYLE, new_style);
                }
            }
        }

        Ok(Self {
            window,
            visible: false,
            event_buffer: Arc::new(Mutex::new(Vec::new())),
            last_cursor_pos: Arc::new(Mutex::new(egui::Pos2::ZERO)),
        })
    }

    // ── Visibility ──────────────────────────────────────────────────────

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        self.window.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    // ── Positioning ─────────────────────────────────────────────────────

    /// Reposition and resize using **logical** coordinates relative to the
    /// parent window's content area.
    pub fn set_frame_logical(&self, parent: &Window, rect: super::LogicalRect) {
        if let Ok(inner_pos) = parent.inner_position() {
            let sx = inner_pos.x + (rect.x * rect.scale) as i32;
            let sy = inner_pos.y + (rect.y * rect.scale) as i32;
            let pw = (rect.w * rect.scale) as u32;
            let ph = (rect.h * rect.scale) as u32;
            self.window
                .set_outer_position(PhysicalPosition::new(sx, sy));
            let _ = self.window.request_inner_size(PhysicalSize::new(pw, ph));
        }
    }

    /// Reposition and resize using **screen-space** coordinates.
    pub fn set_frame(
        &self,
        _parent: &Window,
        screen_x: f32,
        screen_y: f32,
        width: f32,
        height: f32,
        scale: f32,
    ) {
        let px = (screen_x * scale) as i32;
        let py = (screen_y * scale) as i32;
        let pw = (width * scale) as u32;
        let ph = (height * scale) as u32;
        self.window
            .set_outer_position(PhysicalPosition::new(px, py));
        let _ = self.window.request_inner_size(PhysicalSize::new(pw, ph));
    }

    // ── Z-order ─────────────────────────────────────────────────────────

    pub fn ensure_above_overlays(&self) {
        if !self.visible {
            return;
        }
        if let Ok(handle) = self.window.window_handle()
            && let RawWindowHandle::Win32(h) = handle.as_raw()
        {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::*;
                let hwnd_val = h.hwnd.get() as *mut core::ffi::c_void;
                SetWindowPos(
                    hwnd_val,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
            }
        }
    }

    // ── Event monitoring ────────────────────────────────────────────────

    /// No-op on Windows — event monitoring is macOS-specific.
    pub fn install_event_monitor(&mut self, _panel_logical_height: f32) {}

    // ── Platform queries ────────────────────────────────────────────────

    pub fn window_id(&self) -> Option<winit::window::WindowId> {
        Some(self.window.id())
    }
}

// ── HasWindowHandle ─────────────────────────────────────────────────────────

impl HasWindowHandle for Panel {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}
