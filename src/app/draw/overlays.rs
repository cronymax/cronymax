//! Overlay browser, settings overlay, and float tooltip rendering

use crate::app::*;

pub(super) fn render_overlay_browser(state: &mut AppState, event_loop: &ActiveEventLoop) {
    // ── Render child GPU surfaces for overlay browser ──
    // The overlay address bar is rendered on a separate wgpu
    // surface attached to the child panel.  This lets the
    // address bar float above docked native webviews.
    // macOS: separate Metal surface on NSPanel.
    // Windows: separate surface on owned popup winit window.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        // ── Click-to-activate: bring overlay to front on mouse-down ──
        // Check each overlay's event buffer for PointerButton pressed
        // events.  If found and this overlay is not already topmost,
        // bring it to the front of the z-stack.
        {
            let topmost = state.webview_manager.topmost_overlay();
            let mut activate_id: Option<u32> = None;
            for wt in &state.webview_tabs {
                if wt.mode == BrowserViewMode::Overlay
                    && wt.manager.visible
                    && Some(wt.id) != topmost
                {
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    if let Some(overlay) = &wt.manager.overlay
                        && let Ok(buf) = overlay.panel.event_buffer.lock()
                    {
                        let has_click = buf.iter().any(|ev| {
                            matches!(ev, egui::Event::PointerButton { pressed: true, .. })
                        });
                        if has_click {
                            activate_id = Some(wt.id);
                        }
                    }
                }
            }
            if let Some(wid) = activate_id {
                state.webview_manager.bring_to_front(wid);
                // Set as active webview tab.
                if let Some(idx) = state.webview_tabs.iter().position(|wt| wt.id == wid) {
                    state.active_webview = idx;
                    state.ui_state.active_webview = Some(idx);
                }
                handle_ui_action(state, UiAction::BringOverlayToFront(wid), event_loop);
            }
        }

        let mut browser_actions: Vec<(u32, crate::ui::UiAction)> = Vec::new();

        // Compute panel origin in main-window-logical coords for
        // tooltip coordinate mapping.
        let bw_off = state.styles.sizes.border;
        let panel_origin_opt = state
            .ui_state
            .overlay_panel_rect
            .map(|[px, py, _, _]| [px + bw_off, py + bw_off]);

        for wt in &mut state.webview_tabs {
            if wt.mode == BrowserViewMode::Overlay && wt.manager.visible {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if let Some(overlay) = &mut wt.manager.overlay {
                    // Only overwrite the URL when the user is NOT
                    // editing — otherwise their in-progress typing
                    // would be discarded every frame.
                    if !wt.address_bar.editing {
                        wt.address_bar.url = wt.url.clone();
                    }
                    let origin = panel_origin_opt.unwrap_or([0.0, 0.0]);
                    match overlay.render_browser_view(
                        &mut wt.address_bar.url,
                        &mut wt.address_bar.editing,
                        &state.gpu.device,
                        &state.gpu.queue,
                        origin,
                        &state.config,
                    ) {
                        Ok(result) => {
                            let wid = wt.id;
                            for action in result.actions {
                                browser_actions.push((wid, action));
                            }
                            // Collect tooltip (last writer wins).
                            if result.tooltip.is_some() {
                                state.float_panel_state.tooltip = result.tooltip;
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to render overlay browser: {e}");
                        }
                    }
                }
            }
        }
        // Process actions from overlay browser buttons.
        for (wid, action) in browser_actions {
            // Set the active webview to the one that triggered the action.
            if let Some(idx) = state.webview_tabs.iter().position(|wt| wt.id == wid) {
                state.active_webview = idx;
                state.ui_state.active_webview = Some(idx);
            }
            // Replace sentinel 0 with actual webview ID for overlay browser actions.
            let action = match action {
                UiAction::CloseWebview(0) => UiAction::CloseWebview(wid),
                UiAction::WebviewToTab(0) => UiAction::WebviewToTab(wid),
                UiAction::NavigateWebview(url, 0) => UiAction::NavigateWebview(url, wid),
                UiAction::WebviewBack(0) => UiAction::WebviewBack(wid),
                UiAction::WebviewForward(0) => UiAction::WebviewForward(wid),
                UiAction::WebviewRefresh(0) => UiAction::WebviewRefresh(wid),
                other => other,
            };
            handle_ui_action(state, action, event_loop);
        }
    }
}

pub(super) fn render_settings_overlay(state: &mut AppState, event_loop: &ActiveEventLoop) {
    // ── Settings overlay lifecycle & render ─────────────────
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let settings_open = state.settings_state.open;

        if settings_open {
            let scale = state.window.scale_factor() as f32;
            let inner = state.window.inner_size();
            let logical_w = inner.width as f32 / scale;
            let logical_h = inner.height as f32 / scale;
            let margin = 50.0_f32;
            let lx = margin;
            let ly = margin;
            let lw = (logical_w - 2.0 * margin).max(200.0);
            let lh = (logical_h - 2.0 * margin).max(100.0);

            // Lazy-create the Modal for Settings.
            if state.settings_overlay.is_none() {
                match crate::renderer::overlay::ModalPanel::new(
                    &state.window,
                    Some(event_loop),
                    lx,
                    ly,
                    lw,
                    lh,
                    scale,
                ) {
                    Ok(panel) => {
                        let phys_w = (lw * scale).round() as u32;
                        let phys_h = (lh * scale).round() as u32;
                        match crate::renderer::overlay::Modal::new(
                            &state.gpu,
                            panel,
                            phys_w.max(1),
                            phys_h.max(1),
                            scale,
                        ) {
                            Ok(overlay) => {
                                state.settings_overlay = Some(overlay);
                            }
                            Err(e) => {
                                log::warn!("Failed to create Settings Modal: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to create Settings ModalPanel: {e}");
                    }
                }
            }

            // Reposition + resize every frame.
            if let Some(ref mut overlay) = state.settings_overlay {
                overlay.set_frame(&state.window, lx, ly, lw, lh, scale);
                let phys_w = (lw * scale).round() as u32;
                let phys_h = (lh * scale).round() as u32;
                use crate::renderer::overlay::Renderer;
                overlay.resize(&state.gpu.device, phys_w.max(1), phys_h.max(1), scale);
            }

            // Lazy-load providers from config on first open.
            if !state.providers_ui_state.loaded {
                state.providers_ui_state.load_from_config(&state.config);
            }

            // Render Settings UI on the overlay surface.
            // Use .take() to split borrows.
            if let Some(mut overlay) = state.settings_overlay.take() {
                let styles_clone = state.styles.clone();
                let mut pm_guard = state.profile_manager.lock().unwrap();
                let history_cache = state.scheduler_history_cache.clone();

                let render_result =
                    overlay.render(&state.gpu.device, &state.gpu.queue, &state.config, |ctx| {
                        let colors = state.config.resolve_colors();
                        state.settings_state.draw_child(
                            ctx,
                            &styles_clone,
                            &colors,
                            crate::ui::settings::SettingsDrawCtx {
                                general_ui_state: Some(&mut state.general_ui_state),
                                channels_ui_state: Some(&mut state.channels_ui_state),
                                onboarding_wizard_state: state.onboarding_wizard_state.as_mut(),
                                agent_registry: Some(&mut state.agent_registry),
                                agents_ui_state: Some(&mut state.agents_ui_state),
                                profile_manager: Some(&mut *pm_guard),
                                profiles_ui_state: Some(&mut state.profiles_ui_state),
                                providers_ui_state: Some(&mut state.providers_ui_state),
                                task_store: Some(&mut state.task_store),
                                scheduler_ui_state: Some(&mut state.scheduler_ui_state),
                                scheduler_history: &history_cache,
                                skills_panel_state: Some(&mut state.skills_panel_state),
                            },
                        )
                    });

                drop(pm_guard);
                state.settings_overlay = Some(overlay);

                match render_result {
                    Ok(settings_actions) => {
                        for action in settings_actions {
                            handle_ui_action(state, action, event_loop);
                        }
                    }
                    Err(e) => {
                        log::warn!("Settings overlay render failed: {e}");
                    }
                }
                // The overlay uses a separate egui context whose
                // repaint callback isn't wired to the main scheduler.
                // Schedule a deferred repaint so the overlay stays
                // responsive without spinning the render loop at max fps.
                state
                    .scheduler
                    .schedule_repaint_after(std::time::Duration::from_millis(100));
            }

            // Ensure Settings overlay is visible and above browser
            // overlay view.
            if let Some(ref mut overlay) = state.settings_overlay {
                use crate::renderer::overlay::Renderer;
                overlay.set_visible(true);
            }
        } else {
            // Settings closed — destroy the overlay.
            if let Some(ref mut overlay) = state.settings_overlay {
                use crate::renderer::overlay::Renderer;
                overlay.set_visible(false);
            }
            state.settings_overlay = None;
        }
    }
}

pub(super) fn render_float_tooltips(state: &mut AppState, event_loop: &ActiveEventLoop) {
    // ── Float renderer tooltip render/hide cycle ─────────────
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if let Some(ref tip) = state.float_panel_state.tooltip {
            // Lazy-create the Float on first tooltip request.
            if state.float_renderer.is_none() {
                let scale = state.window.scale_factor() as f32;
                match crate::renderer::overlay::FloatPanel::new(
                    &state.window,
                    Some(event_loop),
                    scale,
                ) {
                    Ok(fp) => {
                        match crate::renderer::overlay::Float::new(&state.gpu, fp, 200, 40, scale) {
                            Ok(fr) => {
                                state.float_renderer = Some(fr);
                            }
                            Err(e) => {
                                log::warn!("Failed to create Float: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to create FloatPanel: {e}");
                    }
                }
            }

            // Measure actual tooltip size via egui dry-run, then
            // position, resize, render, and show.
            if let Some(ref mut fr) = state.float_renderer {
                let (tip_w, tip_h) = fr.measure_tooltip(&state.config, &tip.text);
                let scale = state.window.scale_factor() as f32;

                // Convert main-window-logical coords to screen.
                #[cfg(target_os = "macos")]
                if let Some(ns_win) =
                    crate::renderer::platform::macos::ns_window_from_winit(&state.window)
                {
                    let wf = ns_win.frame();
                    let sx = wf.origin.x + tip.screen_x as f64 - tip_w as f64 / 2.0;
                    let sy = wf.origin.y + wf.size.height - tip.screen_y as f64 - tip_h as f64;
                    fr.set_frame(&state.window, sx as f32, sy as f32, tip_w, tip_h, scale);
                }

                #[cfg(target_os = "windows")]
                if let Ok(inner_pos) = state.window.inner_position() {
                    let sx = inner_pos.x as f32 / scale + tip.screen_x - tip_w / 2.0;
                    let sy = inner_pos.y as f32 / scale + tip.screen_y;
                    fr.set_frame(&state.window, sx, sy, tip_w, tip_h, scale);
                }

                let phys_w = (tip_w * scale).round() as u32;
                let phys_h = (tip_h * scale).round() as u32;
                {
                    use crate::renderer::overlay::Renderer;
                    fr.resize(&state.gpu.device, phys_w.max(1), phys_h.max(1), scale);
                }
                if let Err(e) = fr.render_tooltip(
                    &state.gpu.device,
                    &state.gpu.queue,
                    &tip.text,
                    &state.config,
                ) {
                    log::warn!("Float renderer tooltip failed: {e}");
                }
                {
                    use crate::renderer::overlay::Renderer;
                    fr.set_visible(true);
                }
            }

            state.scheduler.mark_dirty();
        } else {
            // No tooltip this frame — hide the float renderer.
            if let Some(ref mut fr) = state.float_renderer {
                use crate::renderer::overlay::Renderer;
                fr.set_visible(false);
            }
        }
    }
}
