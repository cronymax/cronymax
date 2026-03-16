#[cfg(target_os = "macos")]
use std::ptr::NonNull;
#[cfg(target_os = "macos")]
use {
    objc2::MainThreadMarker,
    objc2::MainThreadOnly,
    objc2::rc::Retained,
    objc2_app_kit::{
        NSBackingStoreType, NSColor, NSPanel, NSView, NSWindowOrderingMode, NSWindowStyleMask,
    },
    objc2_foundation::{NSPoint, NSRect, NSSize},
};

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::ui::types::TooltipRequest;

#[derive(Debug, Default)]
pub struct FloatPanelState {
    pub tooltip: Option<TooltipRequest>,
}

impl FloatPanelState {
    pub fn clear(&mut self) {
        self.tooltip = None;
    }
}

/// Single shared non-focusable child window for rendering tooltips and
/// transient dialogs above all overlay webviews.
///
/// On macOS this is a borderless `NSPanel` with `NonactivatingPanel` style,
/// `setLevel(parent.level() + 1)`, and `setIgnoresMouseEvents(true)` so all
/// mouse events pass through to the overlay below.
///
/// On Windows this is an owned popup window with extended styles
/// `WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW`.
///
/// Created lazily on first tooltip request.  Hidden when no tooltip is active.
#[allow(dead_code)]
pub struct FloatPanel {
    #[cfg(target_os = "macos")]
    panel: Retained<NSPanel>,

    #[cfg(target_os = "windows")]
    window: Window,

    /// Whether the panel is currently visible.
    pub visible: bool,
}

#[allow(dead_code)]
impl FloatPanel {
    /// Create the Float Panel as a child of the main window.
    ///
    /// macOS: `NSPanel` with `Borderless | NonactivatingPanel`, level = parent.level() + 1,
    ///        `ignoresMouseEvents = true`, attached via `addChildWindow`.
    /// Windows: winit owned popup with WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW.
    pub fn new(
        parent: &Window,
        #[allow(unused)] event_loop: Option<&ActiveEventLoop>,
        #[allow(unused)] scale: f32,
    ) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let ns_window = crate::renderer::platform::macos::ns_window_from_winit(parent)
                .ok_or_else(|| "Could not get NSWindow from winit window".to_string())?;

            // Start with a small rect; will be repositioned per tooltip.
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));

            let panel = unsafe {
                let mtm =
                    MainThreadMarker::new().expect("FloatPanel must be created on the main thread");
                let style =
                    NSWindowStyleMask::Borderless.union(NSWindowStyleMask::NonactivatingPanel);

                let p = NSPanel::initWithContentRect_styleMask_backing_defer(
                    NSPanel::alloc(mtm),
                    frame,
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                );
                p.setOpaque(false);
                p.setBackgroundColor(Some(&NSColor::clearColor()));
                p.setHasShadow(false);
                p.setMovable(false);

                // Z-order: parent.level() + 1 ensures we float above all
                // overlay ModalPanels (which share the parent's level).
                let parent_level = ns_window.level();
                p.setLevel(parent_level + 1);

                // Click-through: all mouse events pass to the overlay below.
                p.setIgnoresMouseEvents(true);

                // Attach as child so it hides/moves with the parent.
                ns_window.addChildWindow_ordered(&p, NSWindowOrderingMode::Above);

                // Start hidden.
                p.orderOut(None);
                p
            };

            Ok(Self {
                panel,
                visible: false,
            })
        }

        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowAttributesExtWindows;

            let event_loop =
                event_loop.ok_or_else(|| "ActiveEventLoop required on Windows".to_string())?;

            let handle = parent.window_handle().map_err(|e| format!("{e}"))?;
            let hwnd = match handle.as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get(),
                _ => return Err("Expected Win32 window handle".into()),
            };

            let attrs = Window::default_attributes()
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_visible(false) // start hidden
                .with_inner_size(winit::dpi::PhysicalSize::new(1u32, 1u32))
                .with_owner_window(hwnd)
                .with_skip_taskbar(true);

            let window = event_loop
                .create_window(attrs)
                .map_err(|e| format!("Failed to create float panel window: {e}"))?;

            // Apply WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW
            // via raw Win32 FFI since winit doesn't expose these flags.
            {
                use raw_window_handle::HasWindowHandle;
                if let Ok(handle) = window.window_handle()
                    && let RawWindowHandle::Win32(h) = handle.as_raw()
                {
                    #[cfg(target_os = "windows")]
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
            })
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (parent, event_loop, scale);
            Err("Float Panel not supported on this platform".into())
        }
    }

    /// Show or hide the Float Panel.
    pub fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;

        #[cfg(target_os = "macos")]
        {
            if visible {
                self.panel.orderFront(None);
            } else {
                self.panel.orderOut(None);
            }
        }

        #[cfg(target_os = "windows")]
        {
            self.window.set_visible(visible);
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = visible;
        }
    }

    /// Re-order the Float Panel above all overlay ModalPanels.
    ///
    /// Called after each overlay z-stack restack cycle.
    /// macOS: `orderFront` (level already ensures above overlays).
    /// Windows: `SetWindowPos(HWND_TOP, SWP_NOACTIVATE)`.
    pub fn ensure_above_overlays(&self) {
        #[cfg(target_os = "macos")]
        {
            if self.visible {
                self.panel.orderFront(None);
            }
        }

        #[cfg(target_os = "windows")]
        {
            if self.visible {
                use raw_window_handle::HasWindowHandle;
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
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {}
    }

    /// Reposition and resize the Float Panel to match tooltip bounds.
    ///
    /// `screen_x`, `screen_y` are in screen-space logical coordinates.
    /// `width`, `height` are in logical points.
    pub fn set_frame(
        &self,
        #[allow(unused)] parent: &Window,
        #[allow(unused)] screen_x: f32,
        #[allow(unused)] screen_y: f32,
        #[allow(unused)] width: f32,
        #[allow(unused)] height: f32,
        #[allow(unused)] scale: f32,
    ) {
        #[cfg(target_os = "macos")]
        {
            // macOS screen coords: bottom-left origin.
            let frame = NSRect::new(
                NSPoint::new(screen_x as f64, screen_y as f64),
                NSSize::new(width as f64, height as f64),
            );
            self.panel.setFrame_display(frame, true);
        }

        #[cfg(target_os = "windows")]
        {
            let px = (screen_x * scale) as i32;
            let py = (screen_y * scale) as i32;
            let pw = (width * scale) as u32;
            let ph = (height * scale) as u32;
            self.window
                .set_outer_position(winit::dpi::PhysicalPosition::new(px, py));
            let _ = self
                .window
                .request_inner_size(winit::dpi::PhysicalSize::new(pw, ph));
        }
    }
}

// ── FloatPanel HasWindowHandle ──────────────────────────────────────────────

impl HasWindowHandle for FloatPanel {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        #[cfg(target_os = "macos")]
        {
            use raw_window_handle::AppKitWindowHandle;

            let content_view: Retained<NSView> = self
                .panel
                .contentView()
                .expect("FloatPanel NSPanel must have a contentView");
            let ptr = Retained::as_ptr(&content_view) as *mut std::ffi::c_void;
            let handle =
                AppKitWindowHandle::new(NonNull::new(ptr).expect("contentView must not be null"));
            let raw = RawWindowHandle::AppKit(handle);
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }

        #[cfg(target_os = "windows")]
        {
            self.window.window_handle()
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(HandleError::Unavailable)
        }
    }
}

// ── FloatPanel Drop ─────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
impl Drop for FloatPanel {
    fn drop(&mut self) {
        if let Some(parent) = self.panel.parentWindow() {
            parent.removeChildWindow(&self.panel);
        }
        self.panel.orderOut(None);
    }
}
