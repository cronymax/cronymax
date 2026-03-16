//! macOS-specific platform support.

use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use raw_window_handle::RawWindowHandle;
use winit::window::Window;
/// Detect whether the system is currently in dark mode.
///
/// Reads NSApplication.sharedApplication().effectiveAppearance.name and checks
/// if it contains "Dark". Falls back to `true` (dark) on any failure.
pub fn is_dark_mode() -> bool {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSString;

    unsafe {
        // NSApplication.sharedApplication
        let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
        if app.is_null() {
            return true; // fallback to dark
        }
        // effectiveAppearance
        let appearance: *mut AnyObject = msg_send![app, effectiveAppearance];
        if appearance.is_null() {
            return true;
        }
        // name
        let name: Retained<NSString> = msg_send![appearance, name];
        name.to_string().contains("Dark")
    }
}

/// Hint the window to use Metal rendering.
#[allow(dead_code)]
pub fn configure_metal_hints() {
    log::debug!("macOS: Metal backend will be used");
}

/// Get macOS-specific window decoration style.
#[allow(dead_code)]
pub fn titlebar_style() -> &'static str {
    "hidden_inset"
}

/// Check if running on Apple Silicon.
#[allow(dead_code)]
pub fn is_apple_silicon() -> bool {
    cfg!(target_arch = "aarch64")
}

/// Configure the NSWindow appearance for a borderless window:
/// enable shadow and prevent native background-based drag.
///
/// Called after window creation. With `with_titlebar_hidden(true)`, the window
/// uses `NSWindowStyleMask::Borderless` so there is no native titlebar — all
/// mouse events reach egui directly.
pub fn setup_window_appearance(window: &winit::window::Window) {
    use objc2::rc::Retained;
    use objc2_app_kit::NSWindow;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = match window.window_handle() {
        Ok(h) => h,
        Err(_) => return,
    };
    let ns_view = match handle.as_raw() {
        RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut objc2::runtime::AnyObject,
        _ => return,
    };

    let ns_window: Retained<NSWindow> = unsafe {
        let ns_view: &objc2_app_kit::NSView = &*(ns_view as *const objc2_app_kit::NSView);
        match ns_view.window() {
            Some(w) => w,
            None => return,
        }
    };

    // Borderless windows don't have shadow by default — enable it.
    ns_window.setHasShadow(true);

    // Make the window non-opaque so the transparent GPU surface
    // reveals the rounded-corner shape instead of a solid rectangle.
    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&objc2_app_kit::NSColor::clearColor()));

    // Prevent the window background from initiating a drag (we handle drag
    // ourselves via the titlebar drag area widget).
    ns_window.setMovableByWindowBackground(false);

    // Apply rounded-corner clipping via Core Animation layer masking.
    // This clips GPU-rendered content (wgpu surface) to a rounded-rect shape
    // so the window bottom corners appear rounded instead of square.
    unsafe {
        use objc2::msg_send;
        let layer: *mut objc2::runtime::AnyObject = msg_send![ns_view, layer];
        if !layer.is_null() {
            let radius: f64 = 16.0;
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
    }
}

// ── macOS coordinate helpers ────────────────────────────────────────────────

/// Convert a **window-relative** rect (logical points, **top-left** origin)
/// to a **screen** rect (logical points, **bottom-left** origin) suitable for
/// `NSPanel::setFrame_display`.
pub fn window_to_screen_rect(parent: &NSWindow, x: f64, y: f64, w: f64, h: f64) -> NSRect {
    let wf = parent.frame();
    NSRect::new(
        NSPoint::new(wf.origin.x + x, wf.origin.y + wf.size.height - y - h),
        NSSize::new(w, h),
    )
}

/// Obtain the parent `NSWindow` from a winit window via raw-window-handle.
///
/// Returns `None` on non-AppKit handles or if the view has no window.
pub fn ns_window_from_winit(window: &Window) -> Option<Retained<NSWindow>> {
    use winit::raw_window_handle::HasWindowHandle as _;

    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(h) => {
            let ns_view = h.ns_view.as_ptr() as *const NSView;
            unsafe { (*ns_view).window() }
        }
        _ => None,
    }
}

/// Lazily register a `KeyableNSPanel` Objective-C class — an NSPanel subclass
/// whose `-canBecomeKeyWindow` returns `YES`.
///
/// A `Borderless` NSPanel returns `NO` from `canBecomeKeyWindow` because it has
/// no title bar or resize bar.  This prevents the panel from ever becoming the
/// key window, which means WKWebView hosted inside it cannot receive keyboard
/// events for form fields (input, textarea, etc.).
///
/// By creating a subclass that overrides this single method, we allow the panel
/// to accept key window status when the user clicks on a form field, while
/// keeping all other `Borderless | NonactivatingPanel` behaviour intact.
pub fn keyable_panel_class() -> &'static objc2::runtime::AnyClass {
    use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
    use std::sync::OnceLock;

    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = AnyClass::get(c"NSPanel").unwrap();
        let mut builder = ClassBuilder::new(c"KeyableNSPanel", superclass)
            .expect("Failed to create KeyableNSPanel ObjC class");

        extern "C" fn can_become_key(_this: *mut AnyObject, _cmd: Sel) -> Bool {
            Bool::YES
        }

        unsafe {
            builder.add_method(
                objc2::sel!(canBecomeKeyWindow),
                can_become_key as extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
        }

        builder.register()
    })
}

/// Extract egui `Modifiers` from NSEvent modifier flags.
pub fn ns_modifiers(event: &objc2_app_kit::NSEvent) -> egui::Modifiers {
    // Raw modifier bits: Shift=17, Control=18, Option=19, Command=20
    let bits = event.modifierFlags().0;
    egui::Modifiers {
        alt: bits & (1 << 19) != 0,
        ctrl: bits & (1 << 18) != 0,
        shift: bits & (1 << 17) != 0,
        mac_cmd: bits & (1 << 20) != 0,
        command: bits & (1 << 20) != 0,
    }
}

/// Map a macOS virtual key-code to an `egui::Key` (subset used in address bar).
pub fn keycode_to_egui_key(code: u16) -> Option<egui::Key> {
    match code {
        0x00 => Some(egui::Key::A),
        0x06 => Some(egui::Key::Z),
        0x08 => Some(egui::Key::C),
        0x09 => Some(egui::Key::V),
        0x07 => Some(egui::Key::X),
        0x24 => Some(egui::Key::Enter),
        0x30 => Some(egui::Key::Tab),
        0x33 => Some(egui::Key::Backspace),
        0x35 => Some(egui::Key::Escape),
        0x75 => Some(egui::Key::Delete),
        0x7B => Some(egui::Key::ArrowLeft),
        0x7C => Some(egui::Key::ArrowRight),
        0x7D => Some(egui::Key::ArrowDown),
        0x7E => Some(egui::Key::ArrowUp),
        0x73 => Some(egui::Key::Home),
        0x77 => Some(egui::Key::End),
        _ => None,
    }
}

/// Convert an NSEvent targeting a child panel into zero or more `egui::Event`s.
///
/// Coordinate transform: NSEvent `locationInWindow` is in logical points with
/// bottom-left origin.  egui uses top-left origin, so: `egui_y = panel_h - ns_y`.
pub fn nsevent_to_egui(event: &objc2_app_kit::NSEvent, panel_height: f32) -> Vec<egui::Event> {
    use objc2_app_kit::NSEventType;

    let etype = event.r#type();
    let loc = event.locationInWindow();
    let pos = egui::pos2(loc.x as f32, panel_height - loc.y as f32);
    let modifiers = ns_modifiers(event);

    let mut events = Vec::new();

    match etype {
        NSEventType::MouseMoved
        | NSEventType::LeftMouseDragged
        | NSEventType::RightMouseDragged => {
            events.push(egui::Event::PointerMoved(pos));
        }
        NSEventType::LeftMouseDown => {
            events.push(egui::Event::PointerMoved(pos));
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            });
        }
        NSEventType::LeftMouseUp => {
            events.push(egui::Event::PointerMoved(pos));
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            });
        }
        NSEventType::RightMouseDown => {
            events.push(egui::Event::PointerMoved(pos));
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers,
            });
        }
        NSEventType::RightMouseUp => {
            events.push(egui::Event::PointerMoved(pos));
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers,
            });
        }
        NSEventType::ScrollWheel => {
            let dx = event.scrollingDeltaX() as f32;
            let dy = event.scrollingDeltaY() as f32;
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(dx, dy),
                modifiers,
            });
        }
        NSEventType::KeyDown | NSEventType::KeyUp => {
            let pressed = etype == NSEventType::KeyDown;
            let key_code = event.keyCode();

            // ── Clipboard shortcuts (Cmd+C/V/X) ──
            if pressed && modifiers.command {
                match key_code {
                    0x08 => {
                        // Cmd+C → Copy
                        events.push(egui::Event::Copy);
                        return events;
                    }
                    0x09 => {
                        // Cmd+V → Paste from system clipboard
                        if let Some(text) = crate::terminal::input::paste_from_clipboard() {
                            events.push(egui::Event::Paste(text));
                        }
                        return events;
                    }
                    0x07 => {
                        // Cmd+X → Cut
                        events.push(egui::Event::Cut);
                        return events;
                    }
                    _ => {}
                }
            }

            // ── Named key events (arrows, backspace, enter, etc.) ──
            if let Some(key) = keycode_to_egui_key(key_code) {
                events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                });

                // Don't also emit Text for control keys or when Cmd is held
                if !pressed || modifiers.command {
                    return events;
                }

                // For Enter/Tab/Backspace/Delete/Escape/arrows — no Text event
                match key_code {
                    0x24 | 0x30 | 0x33 | 0x35 | 0x75 | 0x7B | 0x7C | 0x7D | 0x7E | 0x73 | 0x77 => {
                        return events;
                    }
                    _ => {}
                }
            }

            // ── Text input (printable characters, KeyDown only) ──
            if pressed
                && !modifiers.command
                && let Some(chars) = event.characters()
            {
                let s = chars.to_string();
                if !s.is_empty() && s.chars().all(|c| !c.is_control()) {
                    events.push(egui::Event::Text(s));
                }
            }
        }
        _ => {}
    }

    events
}
