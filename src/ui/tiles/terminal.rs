//! Terminal pane widget — `TerminalPane` struct + rendering methods.

use super::*;
use crate::ui::widget::{Dirties, Fragment, Widget};

/// Stateful widget for rendering terminal/chat panes.
///
/// Persists across frames in `PaneWidgetStore::terminal`.
#[derive(Debug)]
pub struct TerminalPane {
    pub session_id: SessionId,
    pub cached_layout: Option<TerminalLayout>,
    pub clicked_this_frame: bool,
    /// Measured prompt height from the previous frame (for layout splitting).
    pub cached_prompt_h: f32,
}

impl TerminalPane {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            cached_layout: None,
            clicked_this_frame: false,
            cached_prompt_h: 100.0,
        }
    }

    /// Compute pane flags without calculating pixel rects.
    /// Rects are determined by egui's layout system at render time.
    fn compute_pane_flags(&self, state: &FrameState<'_>) -> TerminalLayout {
        let sid = self.session_id;
        let has_input = state.prompt_editors.get(&sid).is_some_and(|il| il.visible);
        let is_chat_mode = has_input
            && state
                .prompt_editors
                .get(&sid)
                .is_some_and(|il| !il.text.starts_with('$') && !il.text.starts_with(':'));
        let chat_context = if is_chat_mode {
            state
                .session_chats
                .get(&sid)
                .map(|chat| (chat.tokens_used, chat.tokens_limit))
        } else {
            None
        };
        TerminalLayout {
            content_rect: egui::Rect::NOTHING,
            input_rect: egui::Rect::NOTHING,
            has_input,
            is_chat_mode,
            chat_context,
        }
    }

    /// Snapshot cell data from the prompt editor to avoid borrow conflicts during rendering.
    fn snapshot_cells(
        &self,
        state: &FrameState<'_>,
    ) -> (Vec<crate::ui::block::BlockMode>, Vec<(String, String)>) {
        let sid = self.session_id;
        let cells = state
            .prompt_editors
            .get(&sid)
            .map(|il| il.blocks.clone())
            .unwrap_or_default();
        let blocks = state
            .prompt_editors
            .get(&sid)
            .map(|il| {
                il.command_blocks
                    .iter()
                    .map(|b| (b.prompt.clone(), b.cmd.clone()))
                    .collect()
            })
            .unwrap_or_default();
        (cells, blocks)
    }

    /// Render the scrollable cell list (terminal + stream blocks).
    ///
    /// Uses `self.cached_layout` for content_rect dimensions.
    fn render_blocks_scroll(
        &self,
        ui: &mut egui::Ui,
        snapshot: (&[crate::ui::block::BlockMode], &[(String, String)]),
        styles: &Styles,
        colors: &crate::ui::styles::colors::Colors,
        state: &mut FrameState<'_>,
        dirties: &mut Dirties,
    ) {
        let (cells, blocks) = snapshot;
        let content_rect = self.cached_layout.unwrap().content_rect;
        let sid = self.session_id;
        let content_h = content_rect.height();
        let theme = styles;
        let mut pending_star: Option<(u32, u32)> = None;

        // Take mutable reference to prompt editor's blocks for tool_calls_log toggles.
        let prompt_editor_blocks = state
            .prompt_editors
            .get_mut(&sid)
            .map(|pe| &mut pe.blocks as *mut Vec<crate::ui::block::BlockMode>);

        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
            |ui| {
                ui.set_clip_rect(content_rect);
                egui::ScrollArea::vertical()
                    .id_salt(egui::Id::new("pane_blocks_scroll").with(sid))
                    .max_height(content_h)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_width(content_rect.width());
                        let mut empty_log_storage: Vec<crate::ui::block::ToolCallEntry> =
                            Vec::new();
                        let empty_log = &mut empty_log_storage;
                        for (cell_idx, cell) in cells.iter().enumerate() {
                            egui::Frame::new()
                                .fill(egui::Color32::TRANSPARENT)
                                .inner_margin(egui::Margin::same(styles.spacing.medium as i8))
                                .show(ui, |ui| match cell {
                                    crate::ui::block::BlockMode::Terminal {
                                        block_id,
                                        frozen_output,
                                    } => {
                                        let output_text = frozen_output
                                            .as_deref()
                                            .or_else(|| {
                                                state.live_outputs.get(&sid).map(|s| s.as_str())
                                            })
                                            .unwrap_or("");
                                        crate::ui::block::BlockWidget::render_terminal_block(
                                            ui,
                                            *block_id,
                                            blocks,
                                            output_text,
                                            theme,
                                            colors,
                                        )
                                    }
                                    crate::ui::block::BlockMode::Info { id, text } => {
                                        crate::ui::block::BlockWidget::render_info_block(
                                            ui, *id, text, theme, colors,
                                        )
                                    }
                                    crate::ui::block::BlockMode::Stream {
                                        id,
                                        prompt,
                                        response,
                                        is_streaming,
                                        tool_status,
                                        ..
                                    } => {
                                        let tool_calls_log: &mut Vec<
                                            crate::ui::block::ToolCallEntry,
                                        > = if let Some(pe_blocks_ptr) = prompt_editor_blocks {
                                            // SAFETY: We hold &mut state which owns prompt_editors.
                                            // We only access one specific block element.
                                            let pe_blocks = unsafe { &mut *pe_blocks_ptr };
                                            if let Some(crate::ui::block::BlockMode::Stream {
                                                tool_calls_log,
                                                ..
                                            }) = pe_blocks.get_mut(cell_idx)
                                            {
                                                tool_calls_log
                                            } else {
                                                empty_log
                                            }
                                        } else {
                                            empty_log
                                        };
                                        let mut c = crate::ui::block::StreamBlockCtx {
                                            sid,
                                            cell_id: *id,
                                            prompt,
                                            response,
                                            is_streaming: *is_streaming,
                                            tool_status: tool_status.as_deref(),
                                            tool_calls_log,
                                            session_chats: state.session_chats,
                                            starred: false,
                                            on_star_toggle: &mut pending_star,
                                        };
                                        crate::ui::block::BlockWidget::render_stream_block(
                                            ui, &mut c, theme, colors,
                                        )
                                    }
                                });

                            ui.add_space(styles.spacing.small);
                            ui.add(egui::Separator::default());
                        }
                    });
            },
        );

        if let Some((session_id, message_id)) = pending_star {
            dirties.actions.push(UiAction::ToggleStarred {
                session_id,
                message_id,
            });
        }
    }
}

/// Temporary view adapting `TerminalPane` to `Widget<egui::Ui>`.
pub struct TerminalPaneView<'w, 'f> {
    pub widget: &'w mut TerminalPane,
    pub state: &'w mut FrameState<'f>,
}

impl Widget<egui::Ui> for TerminalPaneView<'_, '_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let ui = &mut *f.painter;
        let styles = f.styles;
        let colors = &*f.colors;
        let dirties = &mut *f.dirties;
        let sid = self.widget.session_id;
        let full_rect = ui.available_rect_before_wrap();

        // Track clicks to route keyboard input to the correct split pane.
        if ui.input(|i| i.pointer.any_pressed())
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| full_rect.contains(p))
        {
            f.ui_state.focused_terminal_session = Some(sid);
        }

        let (modes, blocks) = self.widget.snapshot_cells(self.state);
        let flags = self.widget.compute_pane_flags(self.state);

        // ── 1. Prompt editor (bottom of pane, auto-sized) ──
        //
        // Render the prompt in a top-down child UI positioned at the pane
        // bottom edge.  We use the previous frame's measured height to place
        // the rect, then update `cached_prompt_h` with the actual height
        // so it converges within one frame.
        let mut prompt_h = 0.0_f32;
        if flags.has_input {
            let prompt_editors = &mut *self.state.prompt_editors;
            let commands: &[CommandEntry] = &self.state.commands;
            let addr_editing = f.ui_state.address_bar.editing;

            let prev_h = self.widget.cached_prompt_h;
            let prompt_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.min.x, full_rect.max.y - prev_h),
                full_rect.max,
            );
            let layout = TerminalLayout {
                content_rect: egui::Rect::NOTHING,
                input_rect: prompt_rect,
                has_input: true,
                is_chat_mode: flags.is_chat_mode,
                chat_context: flags.chat_context,
            };

            let prompt_resp = ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(prompt_rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
                |prompt_ui| {
                    if let Some(pe) = prompt_editors.get_mut(&sid) {
                        let mut child = Fragment::<egui::Ui> {
                            colors: std::rc::Rc::clone(&f.colors),
                            styles,
                            ui_state: &mut *f.ui_state,
                            dirties: &mut *dirties,
                            painter: prompt_ui,
                        };
                        child.add(crate::ui::prompt::PromptWidget {
                            state: pe,
                            sid,
                            layout,
                            commands,
                            address_bar_editing: addr_editing,
                        });

                        if let Some(item) = pe.last_model_selection.take() {
                            dirties.actions.push(UiAction::SwitchModel {
                                session_id: sid,
                                provider: item.provider_name().to_string(),
                                model: item.model.clone(),
                                display_label: item.display_label.clone(),
                            });
                        }
                    }
                },
            );

            // Measure actual content height for next frame.
            prompt_h = prompt_resp.response.rect.height();
            self.widget.cached_prompt_h = prompt_h;
        }

        // ── 2. Content area (blocks / wgpu terminal / empty state) ──
        let content_rect = egui::Rect::from_min_max(
            full_rect.min,
            egui::pos2(full_rect.max.x, full_rect.max.y - prompt_h),
        );
        let layout = TerminalLayout {
            content_rect,
            input_rect: egui::Rect::NOTHING,
            has_input: flags.has_input,
            is_chat_mode: flags.is_chat_mode,
            chat_context: flags.chat_context,
        };
        self.widget.cached_layout = Some(layout);

        if !flags.has_input {
            let _r = ui.allocate_rect(content_rect, egui::Sense::click_and_drag());
            dirties.tile_rects.push(TileRect::Terminal {
                session_id: sid,
                rect: content_rect,
            });
        } else if modes.is_empty() {
            let _r = ui.allocate_rect(content_rect, egui::Sense::click_and_drag());
            if let Some(info) = self.state.blocks.get(&sid).cloned()
                && let Some(il) = self.state.prompt_editors.get_mut(&sid)
            {
                draw_block_overlays(ui, sid, content_rect, &info, &mut il.command_blocks, styles);
            }
        } else {
            self.widget.render_blocks_scroll(
                ui,
                (&modes, &blocks),
                styles,
                colors,
                self.state,
                dirties,
            );
        }

        // ── 3. DnD split overlay ──
        let is_tab_dragging = egui::DragAndDrop::has_payload_of_type::<TabDragPayload>(ui.ctx());
        if is_tab_dragging {
            let dnd_response = ui.interact(
                full_rect,
                egui::Id::new("pane_dnd").with(sid),
                egui::Sense::drag(),
            );
            handle_pane_dnd(ui, sid, full_rect, &dnd_response, &mut dirties.actions);
        }
    }
}
