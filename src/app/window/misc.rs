//! Secondary window event handlers extracted from window_events.rs

use crate::app::*;

pub(super) fn handle_resize(state: &mut AppState, new_size: winit::dpi::PhysicalSize<u32>) {
    log::debug!("Window resized to {}x{}", new_size.width, new_size.height);
    state.gpu.resize(new_size.width, new_size.height);

    if let Some(ref split) = state.split {
        let (viewport, cols, rows) = split.terminal_grid(
            new_size.width,
            new_size.height,
            &state.renderer.cell_size,
            &state.styles,
        );
        state.viewport = viewport;
        for session in state.sessions.values_mut() {
            session.resize(cols, rows);
        }

        // Update active docked webview bounds.
        if let Some(tab) = state.webview_tabs.get_mut(state.active_webview)
            && tab.mode == BrowserViewMode::Docked
        {
            let bounds = split.webview_bounds(new_size.width, new_size.height, &state.styles);
            tab.manager.set_bounds(bounds);
        }
    } else {
        let (viewport, cols, rows) = ui::compute_single_pane(
            new_size.width,
            new_size.height,
            &state.renderer.cell_size,
            &state.styles,
        );
        state.viewport = viewport;
        for session in state.sessions.values_mut() {
            session.resize(cols, rows);
        }
    }

    // Overlay webviews are repositioned every frame using the
    // actual egui rect (see RedrawRequested handler), so we only
    // need to request a redraw here.

    state
        .renderer
        .update_viewport(&state.gpu.queue, new_size.width, new_size.height);

    state.scheduler.mark_dirty();
}

pub(super) fn handle_focus(state: &mut AppState, focused: bool) {
    // Forward focus change to egui so it can track viewport focus
    // (needed for TextEdit cursor painting, among other things).
    state
        .egui
        .on_window_event(&state.window, &winit::event::WindowEvent::Focused(focused));

    if focused {
        // Re-resolve theme on focus — OS dark/light may have changed.
        state.egui.ctx.set_style(state.config.resolve_egui_style());

        // Show overlay child panels when the app regains focus.
        for wt in &state.webview_tabs {
            if wt.mode == BrowserViewMode::Overlay
                && wt.manager.visible
                && let Some(overlay) = &wt.manager.overlay
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
                    wt.manager
                        .overlay
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
            for wt in &state.webview_tabs {
                if wt.mode == BrowserViewMode::Overlay
                    && wt.manager.visible
                    && let Some(overlay) = &wt.manager.overlay
                {
                    overlay.panel.set_visible(false);
                }
            }
        }
    }
    state.scheduler.mark_dirty();
}

pub(super) fn handle_theme_changed(state: &mut AppState) {
    state.egui.ctx.set_style(state.config.resolve_egui_style());
    state.scheduler.mark_dirty();
}

pub(super) fn handle_scale_change(state: &mut AppState) {
    let scale = state.window.scale_factor();
    for tab in &state.webview_tabs {
        crate::renderer::webview::bridge::send_scale_to_webview(&tab.manager.webview, scale);
    }

    // Update Float surface for new DPI scale.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(ref mut fr) = state.float_renderer {
        use crate::renderer::overlay::Renderer;
        let s = scale as f32;
        let pw = (fr.width * s).round() as u32;
        let ph = (fr.height * s).round() as u32;
        fr.resize(&state.gpu.device, pw.max(1), ph.max(1), s);
    }

    state.scheduler.mark_dirty();
}
