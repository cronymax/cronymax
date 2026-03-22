//! Chat pane — `ChatPane` struct, `ChatPaneView` widget, block scroll, thread summaries, pinned blocks.

use super::*;
use crate::ui::widget::{Fragment, Widget};

/// Stateful widget for chat panes — prompt editor, block list, thread support.
///
/// Persists across frames in `PaneWidgetStore::chat`.
#[derive(Debug)]
pub struct ChatPane {
    pub session_id: SessionId,
    pub cached_layout: Option<TerminalLayout>,
    /// Measured prompt height from the previous frame (for layout splitting).
    pub cached_prompt_h: f32,
    /// Per-block inline filter state: `block_id → (open, query_text)`.
    pub block_filters: std::collections::HashMap<usize, (bool, String)>,
}

impl ChatPane {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            cached_layout: None,
            cached_prompt_h: 100.0,
            block_filters: std::collections::HashMap::new(),
        }
    }

    /// Compute pane flags without calculating pixel rects.
    /// Rects are determined by egui's layout system at render time.
    pub(super) fn compute_pane_flags(
        &self,
        state: &FrameState<'_>,
        sid: SessionId,
    ) -> TerminalLayout {
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
    pub(super) fn snapshot_cells(
        &self,
        state: &FrameState<'_>,
    ) -> (Vec<crate::ui::blocks::Block>, Vec<(String, String)>) {
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
}

/// Chat pane view — renders the chat UI with blocks, prompt, and thread support.
///
/// Dispatched from `Pane::Chat` via `behavior.rs`, while `TerminalPaneView`
/// handles `Pane::Terminal` (raw wgpu PTY rendering).
pub struct ChatPaneView<'w, 'f> {
    pub widget: &'w mut ChatPane,
    pub state: &'w mut FrameState<'f>,
}

impl Widget<egui::Ui> for ChatPaneView<'_, '_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let ui = &mut *f.painter;
        let styles = f.styles;
        let _colors = &*f.colors;
        let dirties = &mut *f.dirties;
        let root_sid = self.widget.session_id;
        let full_rect = ui.available_rect_before_wrap();

        // Track clicks to route keyboard input to the correct split pane.
        if ui.input(|i| i.pointer.any_pressed())
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| full_rect.contains(p))
        {
            f.ui_state.focused_terminal_session = Some(root_sid);
        }

        // ── Thread view: swap rendering target ──
        let thread_sid = f.ui_state.thread_view_map.get(&root_sid).copied();
        let sid = thread_sid.unwrap_or(root_sid);

        // Snapshot cells from the effective session's prompt editor.
        let (modes, blocks) = if thread_sid.is_some() {
            let cells = self
                .state
                .prompt_editors
                .get(&sid)
                .map(|il| il.blocks.clone())
                .unwrap_or_default();
            let cmd_blocks = self
                .state
                .prompt_editors
                .get(&sid)
                .map(|il| {
                    il.command_blocks
                        .iter()
                        .map(|b| (b.prompt.clone(), b.cmd.clone()))
                        .collect()
                })
                .unwrap_or_default();
            (cells, cmd_blocks)
        } else {
            self.widget.snapshot_cells(self.state)
        };
        let flags = self.widget.compute_pane_flags(self.state, sid);

        // ── 1. Prompt editor (bottom) ──
        let prompt_sid = sid;
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
                    if let Some(pe) = prompt_editors.get_mut(&prompt_sid) {
                        let mut child = Fragment::<egui::Ui> {
                            colors: std::rc::Rc::clone(&f.colors),
                            styles,
                            ui_state: &mut *f.ui_state,
                            dirties: &mut *dirties,
                            painter: prompt_ui,
                        };
                        child.add(crate::ui::prompt::PromptWidget {
                            state: pe,
                            sid: prompt_sid,
                            layout,
                            commands,
                            address_bar_editing: addr_editing,
                        });

                        if let Some(item) = pe.last_model_selection.take() {
                            dirties.actions.push(UiAction::SwitchModel {
                                session_id: prompt_sid,
                                provider: item.provider_name().to_string(),
                                model: item.model.clone(),
                                display_label: item.display_label.clone(),
                            });
                        }
                    }
                },
            );

            prompt_h = prompt_resp.response.rect.height();
            self.widget.cached_prompt_h = prompt_h;
        }

        // ── 2. Content area (blocks / empty state / pinned content) ──
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

        if modes.is_empty() {
            let is_thread = thread_sid.is_some();
            let has_pinned = !is_thread
                && self
                    .state
                    .session_chats
                    .get(&sid)
                    .is_some_and(|c| c.pinned_content.is_some());

            if is_thread || has_pinned {
                ui.allocate_new_ui(
                    egui::UiBuilder::new()
                        .max_rect(content_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        ui.set_clip_rect(content_rect);
                        egui::ScrollArea::vertical()
                            .id_salt(egui::Id::new("pane_blocks_scroll").with(sid))
                            .max_height(content_rect.height())
                            .show(ui, |ui| {
                                ui.set_width(content_rect.width());
                                let mut pinned_ctx = crate::ui::widget::Context {
                                    colors: std::rc::Rc::clone(&f.colors),
                                    styles,
                                    ui_state: &mut *f.ui_state,
                                    dirties: &mut *dirties,
                                };
                                if is_thread {
                                    ChatPane::render_pinned_parent_block(
                                        ui,
                                        root_sid,
                                        sid,
                                        self.state,
                                        &mut pinned_ctx,
                                    );
                                } else {
                                    self.render_pinned_content(sid, ui, pinned_ctx);
                                }
                            });
                    },
                );
            } else {
                let _r = ui.allocate_rect(content_rect, egui::Sense::click_and_drag());
                if let Some(info) = self.state.blocks.get(&sid).cloned()
                    && let Some(il) = self.state.prompt_editors.get_mut(&sid)
                {
                    draw_block_overlays(
                        ui,
                        sid,
                        content_rect,
                        &info,
                        &mut il.command_blocks,
                        styles,
                    );
                }
            }
        } else {
            let mut c = crate::ui::widget::Context {
                colors: std::rc::Rc::clone(&f.colors),
                styles,
                ui_state: &mut *f.ui_state,
                dirties: &mut *dirties,
            };
            self.widget.render_blocks_scroll(
                ui,
                (&modes, &blocks),
                root_sid,
                sid,
                &mut c,
                self.state,
            );
        }

        // ── 3. DnD split overlay ──
        let is_tab_dragging = egui::DragAndDrop::has_payload_of_type::<TabDragPayload>(ui.ctx());
        if is_tab_dragging {
            let dnd_response = ui.interact(
                full_rect,
                egui::Id::new("pane_dnd").with(root_sid),
                egui::Sense::drag(),
            );
            handle_pane_dnd(ui, root_sid, full_rect, &dnd_response, &mut dirties.actions);
        }
    }
}

impl<'w, 'f> ChatPaneView<'w, 'f> {
    fn render_pinned_content(
        &mut self,
        sid: u32,
        ui: &mut egui::Ui,
        mut pinned_ctx: crate::ui::widget::Context<'_>,
    ) {
        let content = self
            .state
            .session_chats
            .get(&sid)
            .and_then(|c| c.pinned_content.clone())
            .unwrap_or_default();
        let cell = crate::ui::blocks::Block::PinnedContent { text: content };
        let mut pending_star = None;
        let mut pending_branch = None;
        let mut empty_log = Vec::new();
        let mut block_filters = std::collections::HashMap::new();
        let prompt_editors_ptr =
            self.state.prompt_editors as *const std::collections::HashMap<_, _>;
        let mut render_ctx = crate::ui::blocks::BlockRenderCtx {
            sid,
            cell_idx: 0,
            command_blocks: &[],
            live_output: None,
            session_chats: self.state.session_chats,
            prompt_editor_blocks: None,
            block_filters: &mut block_filters,
            is_thread: false,
            pending_star: &mut pending_star,
            pending_branch: &mut pending_branch,
            empty_log: &mut empty_log,
            prompt_editors: prompt_editors_ptr,
            pinned_parent_root_sid: None,
        };
        pinned_ctx
            .bind::<egui::Ui>(ui)
            .add(crate::ui::blocks::BlockWidget {
                cell: &cell,
                ctx: &mut render_ctx,
            });
    }
}

impl ChatPane {
    /// Render the scrollable cell list (terminal + stream blocks).
    ///
    /// Uses `self.cached_layout` for content_rect dimensions.
    fn render_blocks_scroll(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: (&[crate::ui::blocks::Block], &[(String, String)]),
        root_sid: SessionId,
        effective_sid: SessionId,
        ctx: &mut crate::ui::widget::Context<'_>,
        state: &mut FrameState<'_>,
    ) {
        let (cells, blocks) = snapshot;
        let content_rect = self.cached_layout.unwrap().content_rect;
        let sid = effective_sid;
        let content_h = content_rect.height();
        let styles = ctx.styles;
        let mut pending_star: Option<(u32, u32)> = None;
        let mut pending_branch: Option<(u32, u32)> = None;

        // Determine if the current session is a thread (to hide branch buttons).
        let is_thread = state
            .session_chats
            .get(&sid)
            .is_some_and(|c| c.parent_session_id.is_some());

        // Take mutable reference to prompt editor's blocks for tool_calls_log toggles.
        let prompt_editor_blocks = state
            .prompt_editors
            .get_mut(&sid)
            .map(|pe| &mut pe.blocks as *mut Vec<crate::ui::blocks::Block>);

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

                        // ── Pinned parent block (thread only) ──
                        if is_thread {
                            Self::render_pinned_parent_block(ui, root_sid, sid, state, ctx);
                        }

                        // ── Pinned content block (non-thread, e.g. History / Schedule) ──
                        if !is_thread
                            && let Some(content) = state
                                .session_chats
                                .get(&sid)
                                .and_then(|c| c.pinned_content.clone())
                        {
                            let cell =
                                crate::ui::blocks::Block::PinnedContent { text: content };
                            let mut ps = None;
                            let mut pb = None;
                            let mut el = Vec::new();
                            let mut bf = std::collections::HashMap::new();
                            let pe_ptr =
                                state.prompt_editors as *const std::collections::HashMap<_, _>;
                            let mut rc = crate::ui::blocks::BlockRenderCtx {
                                sid,
                                cell_idx: 0,
                                command_blocks: &[],
                                live_output: None,
                                session_chats: state.session_chats,
                                prompt_editor_blocks: None,
                                block_filters: &mut bf,
                                is_thread: false,
                                pending_star: &mut ps,
                                pending_branch: &mut pb,
                                empty_log: &mut el,
                                prompt_editors: pe_ptr,
                                pinned_parent_root_sid: None,
                            };
                            ctx.bind::<egui::Ui>(ui)
                                .add(crate::ui::blocks::BlockWidget {
                                    cell: &cell,
                                    ctx: &mut rc,
                                });
                            ui.add(egui::Separator::default());
                        }

                        let mut empty_log_storage: Vec<crate::ui::blocks::ToolCallEntry> =
                            Vec::new();
                        let prompt_editors_ptr =
                            state.prompt_editors as *const std::collections::HashMap<_, _>;
                        for (cell_idx, cell) in cells.iter().enumerate() {
                            let mut render_ctx = crate::ui::blocks::BlockRenderCtx {
                                sid,
                                cell_idx,
                                command_blocks: blocks,
                                live_output: state.live_outputs.get(&sid).map(|s| s.as_str()),
                                session_chats: state.session_chats,
                                prompt_editor_blocks,
                                block_filters: &mut self.block_filters,
                                is_thread,
                                pending_star: &mut pending_star,
                                pending_branch: &mut pending_branch,
                                empty_log: &mut empty_log_storage,
                                prompt_editors: prompt_editors_ptr,
                                pinned_parent_root_sid: None,
                            };
                            ctx.bind::<egui::Ui>(ui)
                                .add(crate::ui::blocks::BlockWidget {
                                    cell,
                                    ctx: &mut render_ctx,
                                });

                            ui.add(egui::Separator::default().shrink(if is_thread {
                                styles.spacing.large
                            } else {
                                0.0
                            }));
                        }
                    });
            },
        );

        if let Some((session_id, message_id)) = pending_star {
            ctx.dirties.actions.push(UiAction::ToggleStarred {
                session_id,
                message_id,
            });
        }

        if let Some((session_id, cell_id)) = pending_branch {
            // Check if a thread already exists for this cell.
            let existing_thread = state
                .session_chats
                .get(&session_id)
                .and_then(|c| c.threads.get(&cell_id).copied());
            if let Some(thread_sid) = existing_thread {
                ctx.dirties.actions.push(UiAction::NavigateToThread {
                    root_session_id: session_id,
                    thread_session_id: thread_sid,
                });
            } else {
                ctx.dirties.actions.push(UiAction::SpawnThread {
                    session_id,
                    cell_id,
                });
            }
        }
    }

    /// Render the parent block content pinned at the top of a thread view.
    fn render_pinned_parent_block(
        ui: &mut egui::Ui,
        root_sid: SessionId,
        thread_sid: SessionId,
        state: &mut FrameState<'_>,
        ctx: &mut crate::ui::widget::Context<'_>,
    ) {
        // Find the branch cell_id from the thread's chat state.
        let branch_cell_id = state
            .session_chats
            .get(&thread_sid)
            .and_then(|c| c.branch_cell_id);
        let Some(cell_id) = branch_cell_id else {
            return;
        };

        // Find the parent block with matching cell_id from root session's blocks.
        let parent_block = state.prompt_editors.get(&root_sid).and_then(|pe| {
            pe.blocks.iter().enumerate().find_map(|(idx, b)| match b {
                crate::ui::blocks::Block::Stream { id, .. } if *id == cell_id => {
                    Some((idx, b.clone()))
                }
                _ => None,
            })
        });

        let Some((cell_idx, cell)) = parent_block else {
            return;
        };

        // ── Render parent block via BlockWidget (back button rendered by BlockWidget) ──
        let mut pending_star: Option<(u32, u32)> = None;
        let mut pending_branch: Option<(u32, u32)> = None;
        let mut empty_log: Vec<crate::ui::blocks::ToolCallEntry> = Vec::new();
        let mut block_filters: std::collections::HashMap<usize, (bool, String)> =
            std::collections::HashMap::new();
        let prompt_editor_blocks = state
            .prompt_editors
            .get_mut(&root_sid)
            .map(|pe| &mut pe.blocks as *mut Vec<crate::ui::blocks::Block>);
        let prompt_editors_ptr = state.prompt_editors as *const std::collections::HashMap<_, _>;
        let mut render_ctx = crate::ui::blocks::BlockRenderCtx {
            sid: root_sid,
            cell_idx,
            command_blocks: &[],
            live_output: None,
            session_chats: state.session_chats,
            prompt_editor_blocks,
            block_filters: &mut block_filters,
            is_thread: false,
            pending_star: &mut pending_star,
            pending_branch: &mut pending_branch,
            empty_log: &mut empty_log,
            prompt_editors: prompt_editors_ptr,
            pinned_parent_root_sid: Some(root_sid),
        };
        ctx.bind::<egui::Ui>(ui)
            .add(crate::ui::blocks::BlockWidget {
                cell: &cell,
                ctx: &mut render_ctx,
            });
    }
}
