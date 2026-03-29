//! macOS child window implementation using NSPanel.

use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSView, NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::renderer::panel::LogicalRect;

use super::PanelAttrs;

// ── Panel ──────────────────────────────────────────────────────────────

/// Platform child window backed by a borderless `NSPanel`.
///
/// Configuration (shadow, focusability, click-through, z-level, corner
/// radius) is driven entirely by [`PanelAttrs`] — the panel itself
/// is style-agnostic.
#[allow(dead_code)]
pub struct Panel {
    ns_panel: Retained<NSPanel>,
    pub visible: bool,
    /// Shared event buffer for OS-level event → egui::Event forwarding.
    pub event_buffer: Arc<Mutex<Vec<egui::Event>>>,
    /// Last known cursor position in logical coordinates.
    pub last_cursor_pos: Arc<Mutex<egui::Pos2>>,
    /// Handle to the installed NSEvent local monitor.
    pub monitor_id: Option<Retained<AnyObject>>,
    /// Cached last logical rect to skip redundant position/size calls.
    last_logical_rect: Option<LogicalRect>,
}

impl Panel {
    /// Create a new child panel with the given configuration.
    pub fn new(
        parent: &Window,
        _event_loop: Option<&ActiveEventLoop>,
        rect: LogicalRect,
        attrs: PanelAttrs,
    ) -> Result<Self, String> {
        let ns_window = crate::renderer::platform::macos::ns_window_from_winit(parent)
            .ok_or_else(|| "Could not get NSWindow from winit window".to_string())?;

        let frame = if rect.w > 0.0 && rect.h > 0.0 {
            crate::renderer::platform::macos::window_to_screen_rect(
                &ns_window,
                rect.x as f64,
                rect.y as f64,
                rect.w as f64,
                rect.h as f64,
            )
        } else {
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0))
        };

        let ns_panel = unsafe {
            let mtm = MainThreadMarker::new().expect("Panel must be created on the main thread");
            let mask = NSWindowStyleMask::Borderless.union(NSWindowStyleMask::NonactivatingPanel);

            let p = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                frame,
                mask,
                NSBackingStoreType::Buffered,
                false,
            );
            if attrs.opaque {
                p.setOpaque(true);
                p.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
            } else {
                p.setOpaque(false);
                p.setBackgroundColor(Some(&NSColor::clearColor()));
            }
            p.setMovable(false);
            p.setHasShadow(attrs.shadow);

            if attrs.focusable {
                // Swap the panel's isa to KeyableNSPanel so
                // canBecomeKeyWindow returns YES (required for
                // WKWebView keyboard input).
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
                p.setBecomesKeyOnlyIfNeeded(false);
            }

            if attrs.corner_radius > 0.0 {
                let cv = p.contentView().expect("NSPanel must have a contentView");
                cv.setWantsLayer(true);
                let layer: *mut AnyObject = objc2::msg_send![&*cv, layer];
                if !layer.is_null() {
                    let radius = attrs.corner_radius;
                    let _: () = objc2::msg_send![layer, setCornerRadius: radius];
                    let _: () = objc2::msg_send![layer, setMasksToBounds: true];
                }
            }

            if attrs.click_through {
                p.setIgnoresMouseEvents(true);
            }

            if attrs.level_offset != 0 {
                let parent_level = ns_window.level();
                p.setLevel(parent_level + attrs.level_offset as isize);
            }

            ns_window.addChildWindow_ordered(&p, NSWindowOrderingMode::Above);
            if attrs.initially_visible {
                p.orderFront(None);
            } else {
                p.orderOut(None);
            }

            p
        };

        Ok(Self {
            ns_panel,
            visible: attrs.initially_visible,
            event_buffer: Arc::new(Mutex::new(Vec::new())),
            last_cursor_pos: Arc::new(Mutex::new(egui::Pos2::ZERO)),
            monitor_id: None,
            last_logical_rect: None,
        })
    }

    // ── Visibility ──────────────────────────────────────────────────────

    /// Re-apply layer properties on the content view's backing layer.
    ///
    /// wgpu may replace the content view's original CALayer with a
    /// CAMetalLayer when `Overlay::new()` creates the surface.  Call this
    /// **after** wgpu surface creation so corner-radius, masksToBounds,
    /// and opacity flags land on the *actual* rendering layer.
    pub fn configure_layer(&self, corner_radius: f64, opaque: bool) {
        unsafe {
            let cv = self.ns_panel.contentView().expect("NSPanel must have a contentView");
            cv.setWantsLayer(true);
            let layer: *mut AnyObject = objc2::msg_send![&*cv, layer];
            if !layer.is_null() {
                if corner_radius > 0.0 {
                    let _: () = objc2::msg_send![layer, setCornerRadius: corner_radius];
                    let _: () = objc2::msg_send![layer, setMasksToBounds: true];
                }
                if opaque {
                    let _: () = objc2::msg_send![layer, setOpaque: true];
                }
            }
            // Also walk sublayers — wgpu may nest a CAMetalLayer inside
            // the content view's backing layer.
            let sublayers: *mut AnyObject = objc2::msg_send![layer, sublayers];
            if !sublayers.is_null() {
                let count: usize = objc2::msg_send![sublayers, count];
                for i in 0..count {
                    let sub: *mut AnyObject = objc2::msg_send![sublayers, objectAtIndex: i];
                    if !sub.is_null() {
                        if corner_radius > 0.0 {
                            let _: () = objc2::msg_send![sub, setCornerRadius: corner_radius];
                            let _: () = objc2::msg_send![sub, setMasksToBounds: true];
                        }
                        if opaque {
                            let _: () = objc2::msg_send![sub, setOpaque: true];
                        }
                    }
                }
            }
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if visible {
            self.ns_panel.orderFront(None);
        } else {
            self.ns_panel.orderOut(None);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    // ── Positioning ─────────────────────────────────────────────────────

    /// Reposition and resize using **logical** coordinates relative to the
    /// parent window's content area.
    pub fn set_frame_logical(&mut self, parent: &Window, rect: super::LogicalRect) {
        if self.last_logical_rect == Some(rect) {
            return;
        }
        self.last_logical_rect = Some(rect);
        if let Some(ns_window) = crate::renderer::platform::macos::ns_window_from_winit(parent) {
            let frame = crate::renderer::platform::macos::window_to_screen_rect(
                &ns_window,
                rect.x as f64,
                rect.y as f64,
                rect.w as f64,
                rect.h as f64,
            );
            self.ns_panel.setFrame_display(frame, true);
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
        _scale: f32,
    ) {
        let frame = NSRect::new(
            NSPoint::new(screen_x as f64, screen_y as f64),
            NSSize::new(width as f64, height as f64),
        );
        self.ns_panel.setFrame_display(frame, true);
    }

    // ── Z-order ─────────────────────────────────────────────────────────

    pub fn ensure_above_overlays(&self) {
        if self.visible {
            self.ns_panel.orderFront(None);
        }
    }

    // ── Event monitoring ────────────────────────────────────────────────

    /// Install an NSEvent local monitor that intercepts mouse/keyboard events
    /// targeting this panel and converts them to `egui::Event`s pushed into
    /// the shared `event_buffer`.
    pub fn install_event_monitor(&mut self, panel_logical_height: f32) {
        use objc2_app_kit::{NSEvent, NSEventMask};
        use std::ptr::NonNull as NNPtr;

        let buffer = Arc::clone(&self.event_buffer);
        let panel_ptr = Retained::as_ptr(&self.ns_panel) as usize;
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

            let event_window_ptr = unsafe {
                let mtm = MainThreadMarker::new_unchecked();
                event_ref
                    .window(mtm)
                    .map(|w| Retained::as_ptr(&w) as usize)
                    .unwrap_or(0)
            };
            if event_window_ptr != panel_ptr {
                return event.as_ptr();
            }

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

    // ── Platform queries ────────────────────────────────────────────────

    pub fn window_id(&self) -> Option<winit::window::WindowId> {
        None
    }

    /// No-op on macOS — keyboard focus is handled by the NSPanel key window system.
    pub fn focus(&self) {}

    pub fn ns_panel(&self) -> &NSPanel {
        &self.ns_panel
    }
}

// ── HasWindowHandle ─────────────────────────────────────────────────────────

impl HasWindowHandle for Panel {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let content_view: Retained<NSView> = self
            .ns_panel
            .contentView()
            .expect("NSPanel must have a contentView");
        let ptr = Retained::as_ptr(&content_view) as *mut std::ffi::c_void;
        let handle =
            AppKitWindowHandle::new(NonNull::new(ptr).expect("contentView must not be null"));
        let raw = RawWindowHandle::AppKit(handle);
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

// ── Drop ────────────────────────────────────────────────────────────────────

impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor_id.take() {
            use objc2_app_kit::NSEvent;
            unsafe {
                NSEvent::removeMonitor(&monitor);
            }
        }
        if let Some(parent) = self.ns_panel.parentWindow() {
            parent.removeChildWindow(&self.ns_panel);
        }
        self.ns_panel.orderOut(None);
    }
}
