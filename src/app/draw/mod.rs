//! Frame rendering extracted from app/render.rs

mod overlays;
mod post;
mod term;
mod webviews;

use std::rc::Rc;

use crate::ui::widget::Dirties;

use super::*;

// ─── Frame rendering ─────────────────────────────────────────────────────────

pub(super) fn handle_redraw(state: &mut AppState, event_loop: &ActiveEventLoop) {
    // Process any pending PTY output for all sessions.
    let mut any_exited = Vec::new();
    for (id, session) in state.sessions.iter_mut() {
        session.process_pty_output();
        if session.exited {
            any_exited.push(*id);
        }
    }

    // Sync per-session CWD into prompt editors (for file picker).
    for (id, session) in &state.sessions {
        if let Some(ref cwd) = session.cwd
            && let Some(pe) = state.prompt_editors.get_mut(id)
            && pe.cwd.as_deref() != Some(cwd)
        {
            pe.cwd = Some(cwd.clone());
        }
    }

    for id in &any_exited {
        state.sessions.remove(id);
        tiles::remove_terminal_pane(&mut state.tile_tree, *id);
        state.prompt_editors.remove(id);
        log::info!("Session {} exited", id);
    }
    // Sync tab removal in ui_state
    state.ui_state.tabs.retain(|t| match t {
        TabInfo::Chat { session_id, .. } | TabInfo::Terminal { session_id, .. } => {
            state.sessions.contains_key(session_id)
        }
        _ => true,
    });
    if state.ui_state.active_tab >= state.ui_state.tabs.len() {
        state.ui_state.active_tab = state.ui_state.tabs.len().saturating_sub(1);
    }

    if state.sessions.is_empty() {
        log::info!("All sessions exited, closing window");
        event_loop.exit();
        return;
    }

    let active_sid = match tiles::active_terminal_session(&state.tile_tree) {
        Some(id) => id,
        None => {
            event_loop.exit();
            return;
        }
    };

    state.frame_count += 1;

    let output = match state.gpu.surface.get_current_texture() {
        Ok(t) => t,
        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
            let size = state.window.inner_size();
            state.gpu.resize(size.width, size.height);
            state.scheduler.mark_dirty();
            return;
        }
        Err(wgpu::SurfaceError::OutOfMemory) => {
            log::error!("GPU out of memory, exiting");
            event_loop.exit();
            return;
        }
        Err(e) => {
            log::error!("Surface error: {:?}", e);
            return;
        }
    };

    let colors = state.config.resolve_colors();
    let view = output.texture.create_view(&Default::default());
    let win_size = state.window.inner_size();
    let sw = win_size.width as f32;
    let sh = win_size.height as f32;

    let mut encoder = state
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render-encoder"),
        });

    // 1. Clear to background color.
    // wgpu::Color expects LINEAR values; the sRGB surface format converts
    // linear→sRGB on write automatically.  Apply pow(2.2) to convert the
    // sRGB byte values from Color32 to linear.
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations::default(),
                // ops: wgpu::Operations {
                //     load: wgpu::LoadOp::Clear(wgpu::Color {
                //         r: colors.bg_body[0],
                //         g: colors.bg_body[1],
                //         b: colors.bg_body[2],
                //         a: 1.0,
                //     }),
                //     store: wgpu::StoreOp::Store,
                // },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
    }

    // ── 2. Run egui frame FIRST to collect pane rects ────────
    // Clear transient float-panel state for this frame.
    state.float_panel_state.clear();

    // Sync UiState from app state.
    sync_ui_state(state, active_sid);

    // Run egui frame (generates primitives, does not render yet).
    // Per-pane input lines are now rendered inside pane_ui.
    let theme_clone = state.styles.clone();
    let mut dirties = Dirties::default();
    let mut tile_tree = std::mem::replace(&mut state.tile_tree, tiles::create_initial_tree(0, ""));

    // Build per-session cell metrics for editor-mode cell overlays.
    let block_height = state.renderer.cell_size.height;
    let blocks: HashMap<SessionId, Block> = state
        .sessions
        .iter()
        .map(|(&sid, session)| {
            (
                sid,
                Block {
                    block_height,
                    history_size: session.state.history_size(),
                    screen_lines: session.state.viewport_rows(),
                    display_offset: session.state.display_offset(),
                },
            )
        })
        .collect();

    // Capture live terminal cell output for each session that has an
    // active (non-frozen) terminal cell.
    let live_outputs: HashMap<SessionId, String> = state
        .prompt_editors
        .iter()
        .filter_map(|(&sid, prompt_editor)| {
            if let Some(BlockMode::Terminal {
                block_id,
                frozen_output: None,
            }) = prompt_editor.blocks.last()
                && let Some(block) = prompt_editor.command_blocks.get(*block_id)
                && let Some(session) = state.sessions.get(&sid)
            {
                let abs_start = block.abs_row;
                let abs_end = session.state.abs_cursor_row();
                let text = session.state.capture_text(abs_start + 1, abs_end);
                return Some((sid, text));
            }
            None
        })
        .collect();

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let settings_in_child = state.settings_overlay.is_some();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let settings_in_child = false;

    let mut blocks = Some(blocks);
    let mut live_outputs = Some(live_outputs);

    let (primitives, textures_delta, egui_open_url) = state.egui.run(&state.window, |ctx| {
        let mut fragment = ui::widget::Fragment::new(
            ctx,
            Rc::new(colors),
            &state.config.styles,
            &mut state.ui_state,
            &mut dirties,
        );
        fragment.add(crate::ui::frame::FrameWidget {
            tile_tree: &mut tile_tree,
            prompt_editors: &mut state.prompt_editors,
            blocks: blocks.take().unwrap_or_default(),
            session_chats: &mut state.session_chats,
            live_outputs: live_outputs.take().unwrap_or_default(),
            channel_messages: &state.channel_messages,
            pane_widgets: &mut state.pane_widgets,
            settings_in_child,
            settings_state: &mut state.settings_state,
            theme: &theme_clone,
            profile_manager: &state.profile_manager,
            providers_ui_state: &mut state.providers_ui_state,
            config: &state.config,
            general_ui_state: &mut state.general_ui_state,
            channels_ui_state: &mut state.channels_ui_state,
            onboarding_wizard_state: state.onboarding_wizard_state.as_mut(),
            agent_registry: &mut state.agent_registry,
            agents_ui_state: &mut state.agents_ui_state,
            profiles_ui_state: &mut state.profiles_ui_state,
            task_store: &mut state.task_store,
            scheduler_ui_state: &mut state.scheduler_ui_state,
            scheduler_history: &state.scheduler_history_cache,
            skills_panel_state: &mut state.skills_panel_state,
        });
    });
    state.tile_tree = tile_tree;
    state.tile_rects = dirties.tile_rects.clone();

    // Route docked webview address bar tooltips through the FloatPanel
    // so they render above native WKWebView/WebView2.
    if let Some(tip) = dirties.float_tooltip {
        state.float_panel_state.tooltip = Some(tip);
    }

    // Redirect egui hyperlink clicks (from markdown rendering) to
    // the built-in overlay browser instead of the system browser.
    if let Some(url) = egui_open_url {
        log::info!("Intercepted egui link click → opening in overlay: {}", url);
        open_webview(state, &url, event_loop);
    }

    // Per-pane PTY resize when grid dimensions change.
    let scale = state.window.scale_factor() as f32;
    for tr in &dirties.tile_rects {
        if let tiles::TileRect::Terminal { session_id, rect } = tr {
            let pw = rect.width() * scale;
            let ph = rect.height() * scale;
            let new_cols = (pw / state.renderer.cell_size.width).floor().max(1.0) as u16;
            let new_rows = (ph / state.renderer.cell_size.height).floor().max(1.0) as u16;
            let prev = state.prev_grid_sizes.get(session_id).copied();
            if prev != Some((new_cols, new_rows)) {
                state
                    .prev_grid_sizes
                    .insert(*session_id, (new_cols, new_rows));
                if let Some(session) = state.sessions.get_mut(session_id) {
                    session.resize(new_cols, new_rows);
                }
            }
        }
    }

    // Position webview panes that are embedded in the tile tree.
    // Inset pane bounds that touch window edges so the rounded
    // corners (16px radius) of the window background remain visible.
    let screen_rect = state.egui.ctx.screen_rect();
    let corner_inset = 8.0_f32; // inset in logical px for corner rounding
    let visible_webview_ids: std::collections::HashSet<u32> = dirties
        .tile_rects
        .iter()
        .filter_map(|tr| match tr {
            tiles::TileRect::BrowserView { webview_id, .. } => Some(*webview_id),
            _ => None,
        })
        .collect();
    for tr in &dirties.tile_rects {
        if let tiles::TileRect::BrowserView { webview_id, rect } = tr
            && let Some(wt) = state
                .webview_tabs
                .iter_mut()
                .find(|wt| wt.id == *webview_id)
        {
            wt.manager.set_visible(true);
            // Apply bottom inset if pane touches the window bottom edge.
            let mut r = *rect;
            if (r.max.y - screen_rect.max.y).abs() < 2.0 {
                r.max.y -= corner_inset;
            }
            // Also apply left/right insets at bottom corners.
            if (r.min.x - screen_rect.min.x).abs() < 2.0
                && (r.max.y - screen_rect.max.y).abs() < 2.0 + corner_inset
            {
                r.min.x += corner_inset;
            }
            if (r.max.x - screen_rect.max.x).abs() < 2.0
                && (r.max.y - screen_rect.max.y).abs() < 2.0 + corner_inset
            {
                r.max.x -= corner_inset;
            }
            let bounds = Bounds {
                x: (r.left() * scale) as u32,
                y: (r.top() * scale) as u32,
                width: (r.width() * scale) as u32,
                height: (r.height() * scale) as u32,
            };
            // If the webview lives in an overlay panel (was
            // originally created as a popover and later docked),
            // reposition the panel too.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            let has_overlay = wt.manager.overlay.is_some();
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let has_overlay = false;
            if has_overlay {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if let Some(overlay) = &wt.manager.overlay {
                    overlay.panel.set_frame_logical(
                        &state.window,
                        r.left(),
                        r.top(),
                        r.width(),
                        r.height(),
                        scale,
                    );
                }
                let pb = Bounds::new(
                    0,
                    0,
                    (r.width() * scale) as u32,
                    (r.height() * scale) as u32,
                );
                wt.manager.set_bounds(pb);
            } else {
                wt.manager.set_bounds(bounds);
            }
        }
    }
    // Hide docked webviews that are not in the active tile set.
    for wt in &mut state.webview_tabs {
        if wt.mode == BrowserViewMode::Docked && !visible_webview_ids.contains(&wt.id) {
            wt.manager.set_visible(false);
        }
    }

    webviews::reposition_overlays(state, scale);

    // ── 3. Render egui FIRST so its opaque fills are laid down ──
    // The CentralPanel fill covers the pane area. Terminal text
    // (wgpu) is rendered AFTER egui so it appears on top of the
    // panel background in Terminal / empty-Editor modes.
    let screen_desc = ScreenDescriptor {
        width_px: win_size.width,
        height_px: win_size.height,
        pixels_per_point: state.window.scale_factor() as f32,
    };
    state
        .egui
        .render(crate::renderer::egui_pass::EguiRenderArgs {
            device: &state.gpu.device,
            queue: &state.gpu.queue,
            encoder: &mut encoder,
            color_target: &view,
            primitives: &primitives,
            textures_delta: &textures_delta,
            screen_descriptor: screen_desc,
        });

    term::render_terminal_panes(
        state,
        &dirties.tile_rects,
        &mut encoder,
        &view,
        scale,
        sw,
        sh,
    );

    // Apply window-drag immediately (before GPU submit) so the OS
    // drag loop starts without a full-frame lag.
    let mut deferred_actions = Vec::with_capacity(dirties.actions.len());
    for action in dirties.actions {
        if let UiAction::StartWindowDrag = &action {
            handle_ui_action(state, action, event_loop);
        } else {
            deferred_actions.push(action);
        }
    }

    // Submit and present the main surface.
    state.gpu.queue.submit(std::iter::once(encoder.finish()));
    output.present();

    overlays::render_overlay_browser(state, event_loop);
    overlays::render_settings_overlay(state, event_loop);
    overlays::render_float_tooltips(state, event_loop);

    post::process_post_frame(
        state,
        event_loop,
        &dirties.tile_rects,
        deferred_actions,
        dirties.commands,
    );
}
