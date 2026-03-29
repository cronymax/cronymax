//! Overlay browser, settings overlay, and float tooltip rendering.

use crate::ui::model::AppCtx;
use crate::ui::types::BrowserViewMode;
use crate::ui::{UiAction, ViewMut};
use crate::ui::Ui;

impl Ui {
    /// Render overlay browser address bars on separate wgpu surfaces.
    pub(crate) fn render_overlay_browser(
        &mut self,
        ctx: &mut AppCtx<'_>,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            // ── Click-to-activate: bring overlay to front on mouse-down ──
            {
                let topmost = self.browser_manager.topmost_overlay();
                let mut activate_id: Option<u32> = None;
                for wt in &self.browser_tabs {
                    if wt.mode == BrowserViewMode::Overlay
                        && wt.browser.view.visible
                        && Some(wt.browser.id) != topmost
                    {
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        if let Some(overlay) = &wt.overlay
                            && let Ok(buf) = overlay.panel.event_buffer.lock()
                                && has_pointer_click(&buf) {
                                    activate_id = Some(wt.browser.id);
                                }
                    }
                }
                if let Some(wid) = activate_id {
                    self.browser_manager.bring_to_front(wid);
                    if let Some(idx) = self
                        .browser_tabs
                        .iter()
                        .position(|wt| wt.browser.id == wid)
                    {
                        self.active_browser = idx;
                        ctx.ui_state.active_browser = Some(idx);
                    }
                    self.handle_ui_action(
                        ctx,
                        UiAction::BringOverlayToFront(wid),
                        event_loop,
                    );
                }
            }

            let mut browser_actions: Vec<(u32, UiAction)> = Vec::new();

            for wt in &mut self.browser_tabs {
                if wt.mode == BrowserViewMode::Overlay && wt.browser.view.visible {
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    if let Some(overlay) = &mut wt.overlay {
                        // Skip render when the overlay has no pending input events
                        // and the URL hasn't changed — avoids a full GPU frame.
                        let has_events = overlay
                            .panel
                            .event_buffer
                            .lock()
                            .map(|buf| !buf.is_empty())
                            .unwrap_or(false);
                        let url_changed = !wt.browser.address_bar.editing
                            && wt.browser.address_bar.url != wt.browser.url;
                        if !has_events && !url_changed && !wt.browser.address_bar.editing {
                            continue;
                        }
                        if !wt.browser.address_bar.editing {
                            wt.browser.address_bar.url = wt.browser.url.clone();
                        }
                        let was_editing = wt.browser.address_bar.editing;
                        let r =
                            overlay.render(ctx.config, ctx.ui_state, |mut f| {
                                f.add(crate::ui::browser::BrowserView {
                                    webview_id: wt.browser.id,
                                    url: &mut wt.browser.address_bar.url,
                                    editing: &mut wt.browser.address_bar.editing,
                                    docked: false,
                                });
                            });
                        // When the address bar just gained focus, explicitly
                        // take keyboard focus to prevent WebView2 from
                        // intercepting keyboard events.
                        if wt.browser.address_bar.editing && !was_editing {
                            overlay.panel.focus();
                        }
                        match r {
                            Ok(result) => {
                                let wid = wt.browser.id;
                                for action in result.actions {
                                    browser_actions.push((wid, action));
                                }
                                if let Some(mut tip) = result.float_tooltip {
                                    // Convert overlay-local egui coords to main-window-local coords.
                                    tip.screen_x += wt.overlay_origin.0;
                                    tip.screen_y += wt.overlay_origin.1;
                                    self.float_panel_state.tooltip = Some(tip);
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
                if let Some(idx) = self
                    .browser_tabs
                    .iter()
                    .position(|wt| wt.browser.id == wid)
                {
                    self.active_browser = idx;
                    ctx.ui_state.active_browser = Some(idx);
                }
                let action = match action {
                    UiAction::CloseWebview(0) => UiAction::CloseWebview(wid),
                    UiAction::WebviewToTab(0) => UiAction::WebviewToTab(wid),
                    UiAction::NavigateWebview(url, 0) => UiAction::NavigateWebview(url, wid),
                    UiAction::WebviewBack(0) => UiAction::WebviewBack(wid),
                    UiAction::WebviewForward(0) => UiAction::WebviewForward(wid),
                    UiAction::WebviewRefresh(0) => UiAction::WebviewRefresh(wid),
                    other => other,
                };
                self.handle_ui_action(ctx, action, event_loop);
            }

            // Prevent prompt editor from stealing focus while an overlay
            // address bar is being edited.
            let any_overlay_editing = self.browser_tabs.iter().any(|wt| {
                wt.mode == BrowserViewMode::Overlay
                    && wt.browser.view.visible
                    && wt.browser.address_bar.editing
            });
            if any_overlay_editing {
                ctx.ui_state.address_bar.editing = true;
            }
        }
    }

    /// Render the settings overlay on a child panel surface.
    pub(crate) fn render_settings_overlay(
        &mut self,
        ctx: &mut AppCtx<'_>,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let settings_open = ctx.ui_state.settings_state.open;

            if settings_open {
                let scale = self.frame.window.scale_factor() as f32;
                let inner = self.frame.window.inner_size();
                let logical_w = inner.width as f32 / scale;
                let logical_h = inner.height as f32 / scale;
                let margin = 50.0_f32;
                let lx = margin;
                let ly = margin;
                let lw = (logical_w - 2.0 * margin).max(200.0);
                let lh = (logical_h - 2.0 * margin).max(100.0);

                // Lazy-create the Modal for Settings.
                let mut just_created = false;
                if self.settings_overlay.is_none() {
                    let rect = crate::renderer::panel::LogicalRect {
                        x: lx,
                        y: ly,
                        w: lw,
                        h: lh,
                        scale,
                    };
                    match crate::ui::overlay::Modal::new(
                        &self.frame.window,
                        Some(event_loop),
                        &self.frame.gpu,
                        rect,
                    ) {
                        Ok(overlay) => {
                            self.settings_overlay = Some(overlay);
                            just_created = true;
                        }
                        Err(e) => {
                            log::warn!("Failed to create Settings Modal: {e}");
                        }
                    }
                }

                // Reposition + resize every frame.
                if let Some(ref mut overlay) = self.settings_overlay {
                    let rect = crate::renderer::panel::LogicalRect {
                        x: lx,
                        y: ly,
                        w: lw,
                        h: lh,
                        scale,
                    };
                    overlay.set_frame(&self.frame.window, rect);
                    let phys_w = (lw * scale).round() as u32;
                    let phys_h = (lh * scale).round() as u32;
                    overlay.resize(phys_w.max(1), phys_h.max(1), scale);
                }

                // Lazy-load providers from config on first open.
                if !ctx.ui_state.providers_ui_state.loaded {
                    ctx.ui_state
                        .providers_ui_state
                        .load_from_config(ctx.config);
                }

                // Render Settings UI on the overlay surface.
                // Skip render when the overlay has no pending input events
                // to avoid a full GPU frame on every main-window repaint.
                let has_events = self
                    .settings_overlay
                    .as_ref()
                    .and_then(|o| o.panel.event_buffer.lock().ok())
                    .is_some_and(|buf| !buf.is_empty());
                let needs_render = just_created || has_events;
                // Use .take() to split borrows.
                if needs_render {
                if let Some(mut overlay) = self.settings_overlay.take() {
                    let mut pm_guard = ctx.profile_manager.lock().unwrap();
                    let history_cache = ctx.scheduler_history_cache.to_vec();

                    let render_result =
                        overlay.render(ctx.config, ctx.ui_state, |mut f| {
                            f.add(crate::ui::settings::SettingsModal {
                                agent_registry: Some(ctx.agent_registry),
                                profile_manager: Some(&mut *pm_guard),
                                task_store: Some(ctx.task_store),
                                scheduler_history: &history_cache,
                            })
                        });

                    drop(pm_guard);
                    self.settings_overlay = Some(overlay);

                    match render_result {
                        Ok(dirties) => {
                            for action in dirties.actions {
                                self.handle_ui_action(ctx, action, event_loop);
                            }
                        }
                        Err(e) => {
                            log::warn!("Settings overlay render failed: {e}");
                        }
                    }
                }
                }

                // Ensure Settings overlay is visible and above browser overlay.
                if let Some(ref mut overlay) = self.settings_overlay {
                    overlay.set_visible(true);
                }
            } else {
                // Settings closed — destroy the overlay.
                if let Some(ref mut overlay) = self.settings_overlay {
                    overlay.set_visible(false);
                }
                self.settings_overlay = None;
            }
        }
    }

    /// Render float tooltips on tier-3 child panel.
    pub(crate) fn render_float_tooltips(
        &mut self,
        ctx: &mut AppCtx<'_>,
        #[allow(unused)] event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if let Some(ref tip) = self.float_panel_state.tooltip {
                // Lazy-create the Float on first tooltip request.
                if self.float_renderer.is_none() {
                    let scale = self.frame.window.scale_factor() as f32;
                    match crate::ui::overlay::Float::new(
                        &self.frame.window,
                        Some(event_loop),
                        &self.frame.gpu,
                        200,
                        40,
                        scale,
                    ) {
                        Ok(fr) => {
                            self.float_renderer = Some(fr);
                        }
                        Err(e) => {
                            log::warn!("Failed to create Float: {e}");
                        }
                    }
                }

                if let Some(ref mut fr) = self.float_renderer {
                    let (tip_w, tip_h) = fr.measure_tooltip(ctx.config, &tip.text);
                    let scale = self.frame.window.scale_factor() as f32;

                    #[cfg(target_os = "macos")]
                    if let Some(ns_win) =
                        crate::renderer::platform::macos::ns_window_from_winit(&self.frame.window)
                    {
                        let wf = ns_win.frame();
                        let sx = wf.origin.x + tip.screen_x as f64 - tip_w as f64 / 2.0;
                        let sy =
                            wf.origin.y + wf.size.height - tip.screen_y as f64 - tip_h as f64;
                        fr.set_frame(
                            &self.frame.window,
                            sx as f32,
                            sy as f32,
                            tip_w,
                            tip_h,
                            scale,
                        );
                    }

                    #[cfg(target_os = "windows")]
                    if let Ok(inner_pos) = self.frame.window.inner_position() {
                        let sx = inner_pos.x as f32 / scale + tip.screen_x - tip_w / 2.0;
                        let sy = inner_pos.y as f32 / scale + tip.screen_y;
                        fr.set_frame(&self.frame.window, sx, sy, tip_w, tip_h, scale);
                    }

                    let phys_w = (tip_w * scale).round() as u32;
                    let phys_h = (tip_h * scale).round() as u32;
                    fr.resize(phys_w.max(1), phys_h.max(1), scale);
                    if let Err(e) = fr.render_tooltip(&tip.text, ctx.config, ctx.ui_state) {
                        log::warn!("Float renderer tooltip failed: {e}");
                    }
                    fr.set_visible(true);
                    fr.ensure_above_overlays();
                }
            } else {
                // No tooltip this frame — hide the float renderer.
                if let Some(ref mut fr) = self.float_renderer {
                    fr.set_visible(false);
                }
            }
        }
    }
}

/// Check if an event buffer contains a pointer-button-down event.
pub fn has_pointer_click(events: &[egui::Event]) -> bool {
    events
        .iter()
        .any(|ev| matches!(ev, egui::Event::PointerButton { pressed: true, .. }))
}
