//! Frame rendering — `impl Ui` methods for the per-frame draw cycle.
//!
//! The main entry point is [`Ui::draw_frame`] which runs the egui frame,
//! positions webviews, renders overlays, and returns collected effects for
//! AppState-level post-processing.

mod overlays;
pub mod term;
mod webviews;

use std::collections::HashMap;
use std::rc::Rc;

use winit::event_loop::ActiveEventLoop;

use crate::renderer::scheduler::RenderSchedule;
use crate::renderer::terminal::SessionId;
use crate::renderer::viewport::Viewport;
use crate::ui::Ui;
use crate::ui::UiAction;
use crate::ui::ViewMut;
use crate::ui::blocks::BlockGrid;
use crate::ui::model::AppCtx;
use crate::ui::tiles;
use crate::ui::types::BrowserViewMode;
use crate::ui::widget::{Dirties, Fragment};

/// Result of a successful frame draw — returned to the caller for
/// AppState-level post-processing (command dispatch, chat submission).
pub(crate) struct DrawResult {
    pub actions: Vec<UiAction>,
    pub commands: Vec<(SessionId, String)>,
    pub colon_commands: Vec<String>,
}

impl Ui {
    /// Run the full UI frame: egui render, webview positioning, and overlay
    /// rendering.  Returns `None` on early exit (all sessions gone, GPU error)
    /// or `Some(DrawResult)` with collected actions/commands.
    pub(crate) fn draw_frame(
        &mut self,
        ctx: &mut AppCtx<'_>,
        event_loop: &ActiveEventLoop,
    ) -> Option<DrawResult> {
        *ctx.frame_count += 1;

        // Clear transient float-panel state for this frame.
        self.float_panel_state.clear();

        // Run egui frame.
        let theme_clone = self.styles.clone();
        let mut tile_tree =
            std::mem::replace(&mut self.tile_tree, tiles::create_initial_tree(0, ""));

        // Build per-session cell metrics for editor-mode cell overlays.
        let block_height = ctx.cell_size.height;
        let blocks: HashMap<SessionId, BlockGrid> = ctx
            .sessions
            .iter()
            .map(|(&sid, session)| {
                (
                    sid,
                    BlockGrid {
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
        let live_outputs: HashMap<SessionId, String> = ctx
            .ui_state
            .prompt_editors
            .iter()
            .filter_map(|(&sid, prompt_editor)| {
                if let Some(crate::ui::blocks::Block::Terminal {
                    block_id,
                    frozen_output: None,
                }) = prompt_editor.blocks.last()
                    && let Some(block) = prompt_editor.command_blocks.get(*block_id)
                {
                    // For threads, route to the parent session's PTY.
                    let pty_sid = ctx
                        .session_chats
                        .get(&sid)
                        .and_then(|c| c.parent_session_id)
                        .unwrap_or(sid);
                    if let Some(session) = ctx.sessions.get(&pty_sid) {
                        let abs_start = block.abs_row;
                        let abs_end = session.state.abs_cursor_row();
                        let text = session.state.capture_text(abs_start + 1, abs_end);
                        return Some((sid, text));
                    }
                }
                None
            })
            .collect();

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let settings_in_child = self.settings_overlay.is_some();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let settings_in_child = false;

        // ── Compose egui UI (CPU pass) ────────────────────────────────
        let mut dirties = Dirties::default();
        let full_output = self.run_ui(|c| {
            egui_extras::install_image_loaders(c);
            let colors = ctx.config.resolve_colors();
            let style = ctx.config.styles.build_egui_style(&colors);
            c.set_style(style);

            Fragment::new(
                c,
                Rc::new(colors),
                &ctx.config.styles,
                ctx.ui_state,
                &mut dirties,
            )
            .add(crate::ui::frame::FrameWidget {
                tile_tree: &mut tile_tree,
                blocks: blocks.clone(),
                session_chats: ctx.session_chats,
                live_outputs: live_outputs.clone(),
                channel_messages: ctx.channel_messages,
                settings_in_child,
                theme: &theme_clone,
                profile_manager: ctx.profile_manager,
                config: ctx.config,
                agent_registry: ctx.agent_registry,
                task_store: ctx.task_store,
                scheduler_history: ctx.scheduler_history_cache,
            });

            // Draw toast notifications on top of everything.
            let colors = ctx.config.resolve_colors();
            ctx.ui_state
                .notifications
                .draw(c, &ctx.config.styles, &colors);

            // Draw command palette overlay.
            if let Some(action) = ctx.ui_state.command_palette.draw(c, &ctx.config.styles, &colors) {
                match action {
                    crate::ui::command_palette::PaletteAction::Ui(ui_action) => {
                        dirties.actions.push(ui_action);
                    }
                    crate::ui::command_palette::PaletteAction::ColonCommand(cmd) => {
                        dirties.colon_commands.push(cmd);
                    }
                }
            }
        });

        // ── Update tile layout to current frame ─────────────────────────
        self.tile_tree = tile_tree;
        self.tile_rects = dirties.tile_rects.clone();
        if let Some(tip) = dirties.float_tooltip {
            self.float_panel_state.tooltip = Some(tip);
        }

        // ── Prepare terminal frame (CPU, uses current frame's tile_rects) ─
        let scale = self.frame.window.scale_factor() as f32;
        let terminal_output = self.prepare_terminal_panes(ctx, scale);

        // ── Present (GPU submit) ────────────────────────────────────────
        match self.present(full_output, Some(terminal_output)) {
            Ok(()) => {}
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                let size = self.frame.window.inner_size();
                self.frame.gpu.resize(size.width, size.height);
                ctx.scheduler.mark_dirty();
                return None;
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::error!("GPU out of memory, exiting");
                event_loop.exit();
                return None;
            }
            Err(e) => {
                log::error!("Surface error: {:?}", e);
                return None;
            }
        }

        // Per-pane PTY resize when grid dimensions change.
        let term_pad = ctx.config.styles.spacing.medium;
        for tr in &dirties.tile_rects {
            if let tiles::TileRect::Terminal { session_id, rect } = tr {
                let inner_w = (rect.width() - 2.0 * term_pad).max(0.0);
                let inner_h = (rect.height() - 2.0 * term_pad).max(0.0);
                let new_cols = (inner_w / ctx.cell_size.width).floor().max(1.0) as u16;
                let new_rows = (inner_h / ctx.cell_size.height).floor().max(1.0) as u16;
                let prev = ctx.prev_grid_sizes.get(session_id).copied();
                if prev != Some((new_cols, new_rows)) {
                    ctx.prev_grid_sizes
                        .insert(*session_id, (new_cols, new_rows));
                    if let Some(session) = ctx.sessions.get_mut(session_id) {
                        session.resize(new_cols, new_rows);
                    }
                }
            }
        }

        // Position docked browser-view webviews.
        let sr = self.frame.egui.ctx.screen_rect();
        let corner_inset = ctx.config.styles.spacing.medium;
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
                && let Some(wt) = self
                    .browser_tabs
                    .iter_mut()
                    .find(|wt| wt.browser.id == *webview_id)
            {
                wt.browser.view.set_visible(true);
                let mut r = *rect;
                if (r.max.y - sr.max.y).abs() < 2.0 {
                    r.max.y -= corner_inset;
                }
                if (r.min.x - sr.min.x).abs() < 2.0
                    && (r.max.y - sr.max.y).abs() < 2.0 + corner_inset
                {
                    r.min.x += corner_inset;
                }
                if (r.max.x - sr.max.x).abs() < 2.0
                    && (r.max.y - sr.max.y).abs() < 2.0 + corner_inset
                {
                    r.max.x -= corner_inset;
                }
                let bounds = Viewport {
                    x: (r.left() * scale),
                    y: (r.top() * scale),
                    width: (r.width() * scale),
                    height: (r.height() * scale),
                };
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                let has_overlay = wt.overlay.is_some();
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let has_overlay = false;
                if has_overlay {
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    if let Some(overlay) = &mut wt.overlay {
                        overlay.panel.set_frame_logical(
                            &self.frame.window,
                            crate::renderer::panel::LogicalRect {
                                x: r.left(),
                                y: r.top(),
                                w: r.width(),
                                h: r.height(),
                                scale,
                            },
                        );
                    }
                    wt.overlay_origin = (r.left(), r.top());
                    let pb = Viewport::new(0.0, 0.0, r.width() * scale, r.height() * scale);
                    wt.browser.view.set_viewport(pb);
                } else {
                    wt.browser.view.set_viewport(bounds);
                }
            }
        }
        // Hide docked webviews not in the active tile set.
        for wt in &mut self.browser_tabs {
            if wt.mode == BrowserViewMode::Docked && !visible_webview_ids.contains(&wt.browser.id) {
                wt.browser.view.set_visible(false);
            }
        }

        self.reposition_overlays(ctx, scale);

        // Apply window-drag immediately; defer other actions.
        let mut deferred_actions = Vec::with_capacity(dirties.actions.len());

        // Drain any URL intercepted from egui hyperlink clicks.
        if let Some(url) = self.intercepted_url.take() {
            if let Some(uuid) = url.strip_prefix("cronymax://resume-session/") {
                deferred_actions.push(UiAction::OpenHistorySession(uuid.to_string()));
            } else {
                deferred_actions.push(UiAction::OpenBrowserOverlay(url));
            }
        }

        for action in dirties.actions {
            if let UiAction::StartWindowDrag = &action {
                self.handle_ui_action(ctx, action, event_loop);
            } else {
                deferred_actions.push(action);
            }
        }

        // Overlay rendering.
        self.render_overlay_browser(ctx, event_loop);
        self.render_settings_overlay(ctx, event_loop);
        self.render_float_tooltips(ctx, event_loop);

        Some(DrawResult {
            actions: deferred_actions,
            commands: dirties.commands,
            colon_commands: dirties.colon_commands,
        })
    }
}
