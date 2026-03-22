//! Secondary window event handlers extracted from window_events.rs

use crate::app::*;
use crate::ui::ViewMut;

pub(super) fn handle_resize(state: &mut AppState, new_size: winit::dpi::PhysicalSize<u32>) {
    log::debug!("Window resized to {}x{}", new_size.width, new_size.height);
    state.ui.frame.gpu.resize(new_size.width, new_size.height);

    // Convert physical pixels to logical for grid dimension calc.
    // cell_size is in logical units, so we must use logical window dims.
    let logical = new_size.to_logical::<f32>(state.ui.frame.window.scale_factor());
    let lw = logical.width as u32;
    let lh = logical.height as u32;

    if let Some(ref split) = state.ui.split {
        let (viewport, cols, rows) =
            split.terminal_grid(lw, lh, &state.ui.frame.terminal.cell_size, &state.ui.styles);
        state.ui.viewport = viewport;
        for session in state.sessions.values_mut() {
            session.resize(cols, rows);
        }

        // Update active docked webview bounds.
        if let Some(tab) = state.ui.browser_tabs.get_mut(state.ui.active_browser)
            && tab.mode == BrowserViewMode::Docked
        {
            let bounds = split.webview_bounds(new_size.width, new_size.height, &state.ui.styles);
            tab.browser.view.set_viewport(bounds);
        }
    } else {
        let (viewport, cols, rows) =
            ui::compute_single_pane(lw, lh, &state.ui.frame.terminal.cell_size, &state.ui.styles);
        state.ui.viewport = viewport;
        for session in state.sessions.values_mut() {
            session.resize(cols, rows);
        }
    }

    // Overlay webviews are repositioned every frame using the
    // actual egui rect (see RedrawRequested handler), so we only
    // need to request a redraw here.

    state
        .ui.frame.terminal
        .update_viewport(&state.ui.frame.gpu.queue, new_size.width, new_size.height);

    state.scheduler.mark_dirty();
}

pub(super) fn handle_focus(state: &mut AppState, focused: bool) {
    // Forward focus change to egui so it can track viewport focus
    // (needed for TextEdit cursor painting, among other things).
    state
        .ui.frame
        .on_window_event(&winit::event::WindowEvent::Focused(focused));

    if focused {
        // Re-resolve theme on focus — OS dark/light may have changed.
        state
            .ui.frame
            .egui
            .ctx
            .set_style(state.config.resolve_egui_style());

        // Show overlay child panels when the app regains focus.
        for wt in &mut state.ui.browser_tabs {
            if wt.mode == BrowserViewMode::Overlay
                && wt.browser.view.visible
                && let Some(overlay) = &mut wt.overlay
            {
                overlay.panel.set_visible(true);
            }
        }
    } else {
        // Hide overlay child panels when the app loses focus,
        // so they don't linger above other applications.
        //
        // On macOS, the main window also loses key-window status
        // when a child NSPanel becomes key (e.g. user clicked a
        // form field in an overlay webview).  In that case the
        // app is still active — only hide panels when the app
        // truly deactivates (user switched to another app).
        let app_still_active = {
            #[cfg(target_os = "macos")]
            {
                use objc2::runtime::AnyObject;
                let is_active: bool = unsafe {
                    let app: *mut AnyObject =
                        objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
                    objc2::msg_send![app, isActive]
                };
                is_active
            }
            #[cfg(target_os = "windows")]
            {
                // On Windows, the main window loses focus when a child
                // overlay panel gets focus (e.g. user clicked the address
                // bar).  Check if the foreground window is one of our
                // child overlay windows — if so, the app is still active.
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let fg_hwnd: isize = unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as isize
                };

                state.webview_tabs.iter().any(|wt| {
                    wt.overlay
                        .as_ref()
                        .and_then(|o| o.panel.window_handle().ok())
                        .is_some_and(|wh| match wh.as_raw() {
                            RawWindowHandle::Win32(h) => h.hwnd.get() == fg_hwnd,
                            _ => false,
                        })
                })
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                false
            }
        };
        if !app_still_active {
            for wt in &mut state.ui.browser_tabs {
                if wt.mode == BrowserViewMode::Overlay
                    && wt.browser.view.visible
                    && let Some(overlay) = &mut wt.overlay
                {
                    overlay.panel.set_visible(false);
                }
            }
        }
    }
    state.scheduler.mark_dirty();
}

pub(super) fn handle_theme_changed(state: &mut AppState) {
    state
        .ui.frame
        .egui
        .ctx
        .set_style(state.config.resolve_egui_style());
    state.scheduler.mark_dirty();
}

pub(super) fn handle_scale_change(state: &mut AppState) {
    let scale = state.ui.frame.window.scale_factor();
    for tab in &state.ui.browser_tabs {
        crate::renderer::webview::bridge::send_scale_to_webview(&tab.browser.view.webview, scale);
    }

    // Update Float surface for new DPI scale.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(ref mut fr) = state.ui.float_renderer {
        let s = scale as f32;
        let pw = (fr.width * s).round() as u32;
        let ph = (fr.height * s).round() as u32;
        fr.resize(pw.max(1), ph.max(1), s);
    }

    state.scheduler.mark_dirty();
}

/// Convert a winit `WindowEvent` to zero or more `egui::Event`s.
///
/// `scale` converts physical pixel positions (from winit) to logical points
/// (expected by egui).
#[cfg(target_os = "windows")]
pub fn winit_event_to_egui(event: &winit::event::WindowEvent, scale: f32) -> Vec<egui::Event> {
    use winit::event::{ElementState, MouseButton, WindowEvent as WE};

    let mut out = Vec::new();
    match event {
        WE::CursorMoved { position, .. } => {
            out.push(egui::Event::PointerMoved(egui::pos2(
                position.x as f32 / scale,
                position.y as f32 / scale,
            )));
        }
        WE::MouseInput {
            state: st, button, ..
        } => {
            let btn = match button {
                MouseButton::Left => egui::PointerButton::Primary,
                MouseButton::Right => egui::PointerButton::Secondary,
                MouseButton::Middle => egui::PointerButton::Middle,
                _ => return out,
            };
            let pressed = *st == ElementState::Pressed;
            out.push(egui::Event::PointerButton {
                pos: egui::Pos2::ZERO,
                button: btn,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        WE::MouseWheel { delta, .. } => {
            use winit::event::MouseScrollDelta;
            let (dx, dy) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x * 24.0, *y * 24.0),
                MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
            };
            out.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(dx, dy),
                modifiers: egui::Modifiers::NONE,
            });
        }
        WE::KeyboardInput { event: key_ev, .. } => {
            if key_ev.state == ElementState::Pressed
                && let Some(text) = &key_ev.text
            {
                let s = text.as_str();
                if !s.is_empty() && !s.chars().next().is_some_and(|c| c.is_control()) {
                    out.push(egui::Event::Text(s.to_string()));
                }
            }
        }
        _ => {}
    }
    out
}
