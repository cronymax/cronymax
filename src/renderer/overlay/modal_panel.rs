#[cfg(target_os = "macos")]
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "macos")]
use {
    objc2::MainThreadMarker,
    objc2::MainThreadOnly,
    objc2::rc::Retained,
    objc2::runtime::AnyObject,
    objc2_app_kit::{
        NSBackingStoreType, NSColor, NSPanel, NSView, NSWindowOrderingMode, NSWindowStyleMask,
    },
};

#[cfg(target_os = "windows")]
use winit::dpi::{PhysicalPosition, PhysicalSize};

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub struct ModalPanel {
    #[cfg(target_os = "macos")]
    panel: Retained<NSPanel>,

    /// Shared event buffer for OS-level event → egui::Event forwarding.
    /// On macOS the NSEvent monitor closure pushes events; on Windows the
    /// winit WindowEvent routing pushes events.  `render_browser()` drains them.
    pub event_buffer: Arc<Mutex<Vec<egui::Event>>>,

    /// Last known cursor position in logical coordinates.
    /// Persists across event buffer drains so that PointerButton events
    /// (which don't carry a position on Windows) can be patched correctly.
    pub last_cursor_pos: Arc<Mutex<egui::Pos2>>,

    /// Handle to the installed NSEvent local monitor, used for cleanup in Drop.
    #[cfg(target_os = "macos")]
    pub monitor_id: Option<Retained<AnyObject>>,

    #[cfg(target_os = "windows")]
    window: Window,
}

impl ModalPanel {
    /// Create a new child panel floating above `parent`.
    ///
    /// * `parent`     – the main application window.
    /// * `event_loop` – required on Windows for winit window creation (ignored elsewhere).
    /// * `lx, ly, lw, lh` – logical coordinates relative to the parent content area.
    /// * `scale`      – DPI scale factor.
    pub fn new(
        parent: &Window,
        #[allow(unused)] event_loop: Option<&ActiveEventLoop>,
        lx: f32,
        ly: f32,
        lw: f32,
        lh: f32,
        #[allow(unused)] scale: f32,
    ) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let ns_window = crate::renderer::platform::macos::ns_window_from_winit(parent)
                .ok_or_else(|| "Could not get NSWindow from winit window".to_string())?;
            let frame = crate::renderer::platform::macos::window_to_screen_rect(
                &ns_window, lx as f64, ly as f64, lw as f64, lh as f64,
            );

            let panel = unsafe {
                let mtm =
                    MainThreadMarker::new().expect("ModalPanel must be created on the main thread");
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
                // Native shadow — platform API handles the drop shadow
                // so the main window doesn't need to render one.
                p.setHasShadow(true);
                p.setMovable(false);

                // ── Make the panel capable of becoming the key window ──
                //
                // Problem: A Borderless NSPanel returns NO from
                // -canBecomeKeyWindow (no title bar / resize bar), so it can
                // NEVER become key.  Without key-window status, WKWebView
                // form fields cannot receive keyboard events.
                //
                // Fix: Swap the panel's isa to KeyableNSPanel, a runtime
                // subclass that overrides -canBecomeKeyWindow → YES.
                let cls = crate::renderer::platform::macos::keyable_panel_class();
                unsafe extern "C" {
                    fn object_setClass(
                        obj: *mut std::ffi::c_void,
                        cls: *const std::ffi::c_void,
                    ) -> *const std::ffi::c_void;
                }
                object_setClass(
                    Retained::as_ptr(&p) as *const _ as *mut std::ffi::c_void,
                    cls as *const objc2::runtime::AnyClass as *const std::ffi::c_void,
                );

                // With canBecomeKeyWindow now returning YES, this flag
                // controls WHEN the panel becomes key.  false = always accept
                // key status on click (WKWebView's internal fields don't
                // trigger the "needs keyboard" heuristic that true relies on).
                p.setBecomesKeyOnlyIfNeeded(false);

                // Round the contentView layer so ALL subviews (Metal surface,
                // WKWebView) are clipped to rounded corners.
                let cv = p.contentView().expect("NSPanel must have a contentView");
                cv.setWantsLayer(true);
                let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![&*cv, layer];
                if !layer.is_null() {
                    let radius: f64 = 8.0;
                    let _: () = objc2::msg_send![layer, setCornerRadius: radius];
                    let _: () = objc2::msg_send![layer, setMasksToBounds: true];
                }

                ns_window.addChildWindow_ordered(&p, NSWindowOrderingMode::Above);
                p.orderFront(None);
                p
            };

            Ok(Self {
                panel,
                event_buffer: Arc::new(Mutex::new(Vec::new())),
                last_cursor_pos: Arc::new(Mutex::new(egui::Pos2::ZERO)),
                monitor_id: None,
            })
            .map(|mut s| {
                s.install_event_monitor(lh);
                s
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

            let inner_pos = parent.inner_position().map_err(|e| format!("{e}"))?;
            let sx = inner_pos.x + (lx * scale) as i32;
            let sy = inner_pos.y + (ly * scale) as i32;
            let pw = (lw * scale) as u32;
            let ph = (lh * scale) as u32;

            let attrs = Window::default_attributes()
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_visible(false)
                .with_inner_size(PhysicalSize::new(pw, ph))
                .with_position(PhysicalPosition::new(sx, sy))
                .with_owner_window(hwnd)
                .with_skip_taskbar(true);

            let window = event_loop
                .create_window(attrs)
                .map_err(|e| format!("Failed to create child window: {e}"))?;

            Ok(Self {
                window,
                event_buffer: Arc::new(Mutex::new(Vec::new())),
                last_cursor_pos: Arc::new(Mutex::new(egui::Pos2::ZERO)),
            })
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (parent, event_loop, lx, ly, lw, lh, scale);
            Err("Child panel windows not supported on this platform".into())
        }
    }

    /// Reposition and resize the panel using **logical** coordinates relative
    /// to the parent window's content area.
    pub fn set_frame_logical(
        &self,
        parent: &Window,
        lx: f32,
        ly: f32,
        lw: f32,
        lh: f32,
        #[allow(unused)] scale: f32,
    ) {
        #[cfg(target_os = "macos")]
        if let Some(ns_window) = crate::renderer::platform::macos::ns_window_from_winit(parent) {
            let frame = crate::renderer::platform::macos::window_to_screen_rect(
                &ns_window, lx as f64, ly as f64, lw as f64, lh as f64,
            );
            self.panel.setFrame_display(frame, true);
        }

        #[cfg(target_os = "windows")]
        if let Ok(inner_pos) = parent.inner_position() {
            let sx = inner_pos.x + (lx * scale) as i32;
            let sy = inner_pos.y + (ly * scale) as i32;
            let pw = (lw * scale) as u32;
            let ph = (lh * scale) as u32;
            self.window
                .set_outer_position(PhysicalPosition::new(sx, sy));
            let _ = self.window.request_inner_size(PhysicalSize::new(pw, ph));
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (parent, lx, ly, lw, lh, scale);
        }
    }

    /// Show or hide the panel.
    pub fn set_visible(&self, visible: bool) {
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

    /// Whether the panel is currently visible.
    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.panel.isVisible()
        }

        #[cfg(target_os = "windows")]
        {
            self.window.is_visible().unwrap_or(false)
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            false
        }
    }

    /// Install an NSEvent local monitor that intercepts mouse/keyboard events
    /// targeting this panel and converts them to `egui::Event`s pushed into
    /// the shared `event_buffer`.
    ///
    /// Called once after panel creation.  The monitor handle is stored in
    /// `self.monitor_id` for cleanup in `Drop`.
    #[cfg(target_os = "macos")]
    fn install_event_monitor(&mut self, panel_logical_height: f32) {
        use objc2_app_kit::{NSEvent, NSEventMask};
        use std::ptr::NonNull as NNPtr;

        let buffer = Arc::clone(&self.event_buffer);
        // Capture a raw pointer to the panel for identity comparison in the
        // monitor closure.  We compare raw pointers, so no Retained is needed
        // inside the closure (avoids reference cycle).
        let panel_ptr = Retained::as_ptr(&self.panel) as usize;
        let panel_height = panel_logical_height;

        let mask = NSEventMask::LeftMouseDown
            .union(NSEventMask::LeftMouseUp)
            .union(NSEventMask::RightMouseDown)
            .union(NSEventMask::RightMouseUp)
            .union(NSEventMask::MouseMoved)
            .union(NSEventMask::LeftMouseDragged)
            .union(NSEventMask::RightMouseDragged)
            .union(NSEventMask::ScrollWheel)
            .union(NSEventMask::KeyDown)
            .union(NSEventMask::KeyUp);

        let block = block2::RcBlock::new(move |event: NNPtr<NSEvent>| -> *mut NSEvent {
            let event_ref: &NSEvent = unsafe { event.as_ref() };

            // Check if this event targets our panel by comparing window pointers.
            let event_window_ptr = unsafe {
                let mtm = MainThreadMarker::new_unchecked();
                event_ref
                    .window(mtm)
                    .map(|w| Retained::as_ptr(&w) as usize)
                    .unwrap_or(0)
            };
            if event_window_ptr != panel_ptr {
                // Not our panel — pass through.
                return event.as_ptr();
            }

            // NOTE: We do NOT call makeKeyWindow here.  The NSPanel's
            // sendEvent: handles key-window promotion automatically when
            // canBecomeKeyWindow=YES + becomesKeyOnlyIfNeeded=NO.  Calling
            // makeKeyWindow from within the event monitor (which fires
            // BEFORE sendEvent:) causes a premature focus change that
            // triggers the main window's Focused(false) handler — which
            // hides all overlay panels, creating flicker.

            // Convert NSEvent to egui::Event(s).
            let egui_events =
                crate::renderer::platform::macos::nsevent_to_egui(event_ref, panel_height);

            if !egui_events.is_empty()
                && let Ok(mut buf) = buffer.lock()
            {
                buf.extend(egui_events);
            }

            event.as_ptr()
        });

        let monitor =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        self.monitor_id = monitor;
    }

    /// Get the winit `WindowId` for this child panel.
    ///
    /// Returns `Some` on Windows (where the child panel is a real winit Window),
    /// `None` on macOS (where the panel is an NSPanel without a winit WindowId).
    #[allow(dead_code)]
    pub fn window_id(&self) -> Option<winit::window::WindowId> {
        #[cfg(target_os = "windows")]
        {
            Some(self.window.id())
        }

        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    /// Get a reference to the underlying NSPanel (macOS only).
    #[cfg(target_os = "macos")]
    pub fn panel(&self) -> &NSPanel {
        &self.panel
    }
}

// ── HasWindowHandle ─────────────────────────────────────────────────────────

impl HasWindowHandle for ModalPanel {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        #[cfg(target_os = "macos")]
        {
            use raw_window_handle::AppKitWindowHandle;

            let content_view: Retained<NSView> = self
                .panel
                .contentView()
                .expect("NSPanel must have a contentView");
            let ptr = Retained::as_ptr(&content_view) as *mut std::ffi::c_void;
            let handle =
                AppKitWindowHandle::new(NonNull::new(ptr).expect("contentView must not be null"));
            let raw = RawWindowHandle::AppKit(handle);
            // SAFETY: content_view is alive as long as `self` (the panel owns it).
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

// ── Drop ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
impl Drop for ModalPanel {
    fn drop(&mut self) {
        // Remove the NSEvent monitor to prevent event leaks.
        if let Some(monitor) = self.monitor_id.take() {
            use objc2_app_kit::NSEvent;
            unsafe {
                NSEvent::removeMonitor(&monitor);
            }
        }
        if let Some(parent) = self.panel.parentWindow() {
            parent.removeChildWindow(&self.panel);
        }
        self.panel.orderOut(None);
    }
}
