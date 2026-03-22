//! Block types and widgets — content units displayed inside a pane (§2.3).
//!
//! Each pane contains an ordered list of [`Block`] entries rendered in
//! chronological order:
//! - **Terminal block** (§2.3.1): PTY command + captured output
//! - **Stream block** (§2.3.2): SSE/LLM exchange with streaming, tool calls,
//!   and incremental markdown rendering
//! - **Info block**: plain informational message
//!
//! Each variant lives in its own submodule with a dedicated widget struct:
//! - [`TerminalBlock`] in [`terminal_block`]
//! - [`StreamBlock`] in [`stream`]
//! - [`InfoBlock`] in [`info`]

pub mod info;
pub mod stream;
pub mod terminal;

use crate::renderer::terminal::SessionId;
use crate::ui::UiAction;
use crate::ui::chat::SessionChat;
use crate::ui::prompt::PromptState;
use crate::ui::widget::{Fragment, Widget};

pub use info::InfoBlock;
pub use stream::{StreamBlock, StreamBlockCtx};
pub use terminal::{TerminalBlock, TerminalBlockCtx};

// ─── Block Mode ─────────────────────────────────────────────────────────────

/// A content block in the unified pane cell list — see widget hierarchy §2.3.
///
/// - [`Block::Terminal`] (§2.3.1): PTY command + output
/// - [`Block::Stream`] (§2.3.2): SSE/LLM exchange
#[derive(Debug, Clone)]
pub enum Block {
    /// Terminal (PTY) block (§2.3.1) — command + PTY output.
    Terminal {
        /// Index into `InputLineState::command_blocks`.
        block_id: usize,
        /// Frozen output text (captured when the next block is created).
        /// While `None`, this is the "live" block rendered by wgpu.
        frozen_output: Option<String>,
    },
    /// Informational message block — italic, muted, no prompt marker or star.
    Info {
        /// Unique block ID (per session).
        id: u32,
        /// The info message text.
        text: String,
    },
    /// Stream (SSE) block (§2.3.2) — user prompt + LLM response with SSE streaming,
    /// tool-call details, and incremental markdown rendering.
    Stream {
        /// Unique block ID (per session).
        id: u32,
        /// The user's prompt text (without `?` prefix).
        prompt: String,
        /// Accumulated response text (built up via streaming tokens).
        response: String,
        /// Whether this block is currently streaming.
        is_streaming: bool,
        /// Status text shown while a tool is being invoked (e.g. "Calling open_webview...").
        tool_status: Option<String>,
        /// Log of tool invocations executed during this stream block,
        /// displayed as collapsible detail sections.
        tool_calls_log: Vec<ToolCallEntry>,
    },
    /// Pinned markdown content card (e.g. History or info-only tabs).
    PinnedContent {
        /// The markdown text to render.
        text: String,
    },
}

/// A single tool invocation record displayed in the stream block.
#[derive(Debug, Clone)]
pub struct ToolCallEntry {
    /// Human-readable tool name (e.g. "terminal_execute_command").
    pub name: String,
    /// Summary of the invocation (e.g. "`cat ~/.ssh/id_ed25519.pub`").
    pub summary: String,
    /// Result output (truncated for display).
    pub result: Option<String>,
    /// Whether this entry is expanded (showing result).
    pub expanded: bool,
    /// Whether the tool call is still in progress.
    pub in_progress: bool,
}

/// Per-session terminal grid information required to position cell overlays.
#[derive(Debug, Clone)]
pub struct BlockGrid {
    /// Logical height of one terminal text row in pixels.
    pub block_height: f32,
    /// Number of scrollback history lines currently stored.
    pub history_size: usize,
    /// Number of visible screen lines (pane's current row count).
    pub screen_lines: usize,
    /// Lines scrolled above the bottom of the scrollback (0 = not scrolled).
    pub display_offset: usize,
}

// ─── Utility ────────────────────────────────────────────────────────────────

/// Collect the copyable text content from a block.
pub fn block_text_content(
    mode: &Block,
    blocks: &[(String, String)],
    live_output: Option<&str>,
) -> String {
    match mode {
        Block::Terminal {
            block_id,
            frozen_output,
        } => {
            let (prompt, cmd) = blocks
                .get(*block_id)
                .map(|(p, c)| (p.as_str(), c.as_str()))
                .unwrap_or(("", ""));
            let output = frozen_output.as_deref().or(live_output).unwrap_or("");
            if output.is_empty() {
                format!("{} {}", prompt, cmd)
            } else {
                format!("{} {}\n{}", prompt, cmd, output)
            }
        }
        Block::Info { text, .. } => text.clone(),
        Block::Stream {
            prompt, response, ..
        } => {
            if response.is_empty() {
                format!("? {}", prompt)
            } else {
                format!("? {}\n{}", prompt, response)
            }
        }
        Block::PinnedContent { text } => text.clone(),
    }
}

// ─── Block Widget (Widget<egui::Ui>) ────────────────────────────────────────

/// Per-cell rendering context passed to [`BlockWidget`].
pub struct BlockRenderCtx<'a> {
    /// Effective session ID.
    pub sid: SessionId,
    /// Cell index in the block list (for tool_calls_log access).
    pub cell_idx: usize,
    /// Command blocks snapshot (prompt, command pairs).
    pub command_blocks: &'a [(String, String)],
    /// Live streaming output for the session.
    pub live_output: Option<&'a str>,
    /// Session chats (for threads, stars, markdown caches).
    pub session_chats: &'a mut std::collections::HashMap<SessionId, SessionChat>,
    /// Raw pointer to prompt editor's blocks for tool_calls_log toggles.
    /// SAFETY: Only one element is accessed at a time; caller ensures no aliasing.
    pub prompt_editor_blocks: Option<*mut Vec<Block>>,
    /// Per-block inline filter state: `block_id → (open, query_text)`.
    pub block_filters: &'a mut std::collections::HashMap<usize, (bool, String)>,
    /// Whether the current session is a thread.
    pub is_thread: bool,
    /// Output: pending star toggle (session_id, message_id).
    pub pending_star: &'a mut Option<(u32, u32)>,
    /// Output: pending branch action (session_id, cell_id).
    pub pending_branch: &'a mut Option<(u32, u32)>,
    /// Fallback storage for tool_calls_log when prompt_editor_blocks is unavailable.
    pub empty_log: &'a mut Vec<ToolCallEntry>,
    /// Read-only access to all prompt editors (for thread summary).
    /// SAFETY: Only read through this pointer; mutable writes go through `prompt_editor_blocks`.
    pub prompt_editors: *const std::collections::HashMap<SessionId, PromptState>,
    /// When `Some(root_sid)`, this block is a pinned parent in a thread view.
    /// BlockWidget renders a "back to parent" header above the block content.
    pub pinned_parent_root_sid: Option<SessionId>,
}

/// Inline widget that renders a single [`Block`] cell.
///
/// Encapsulates the per-cell match on `Block` variants, delegating to
/// [`TerminalBlock`], [`InfoBlock`], or [`StreamBlock`] as appropriate.
pub struct BlockWidget<'a, 'b> {
    pub cell: &'a Block,
    pub ctx: &'b mut BlockRenderCtx<'a>,
}

impl Widget<egui::Ui> for BlockWidget<'_, '_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let styles = f.styles;
        let colors_rc = std::rc::Rc::clone(&f.colors);
        let dirties = &mut *f.dirties;
        let ui = &mut *f.painter;
        let colors = &*colors_rc;
        let ctx = &mut *self.ctx;

        // ── Block content ──
        egui::Frame::new()
            .fill(egui::Color32::TRANSPARENT)
            .outer_margin(egui::Margin::same(styles.spacing.medium as i8))
            .inner_margin(egui::Margin::same(styles.spacing.medium as i8))
            .show(ui, |ui| {
                ctx.render_pinned_label_bar(ui, dirties, styles, colors);
                ctx.render_block_content(ui, self.cell, styles, colors);

                // ── Thread summary (after stream blocks) ──
                if let Block::Stream { id, .. } = self.cell {
                    let thread_sid = ctx
                        .session_chats
                        .get(&ctx.sid)
                        .and_then(|c| c.threads.get(id).copied());
                    if let Some(thread_sid) = thread_sid {
                        // SAFETY: We only read other sessions' blocks; mutable
                        // writes go through prompt_editor_blocks (current session only).
                        let prompt_editors = unsafe { &*ctx.prompt_editors };
                        render_thread_summary(ui, thread_sid, prompt_editors, styles, colors);
                    }
                }
            });
    }
}

impl BlockRenderCtx<'_> {
    fn render_pinned_label_bar(
        &mut self,
        ui: &mut egui::Ui,
        dirties: &mut super::widget::Dirties,
        styles: &super::styles::Styles,
        colors: &super::styles::colors::Colors,
    ) {
        // ── Pinned parent back-button header ──
        let Some(root_sid) = self.pinned_parent_root_sid else {
            return;
        };

        ui.horizontal(|ui| {
            let back_btn = ui.add(
                egui::Button::new(
                    egui::RichText::new(crate::ui::i18n::t("chat.back_to_parent"))
                        .color(colors.primary)
                        .size(styles.typography.body0),
                )
                .frame(false),
            );
            if back_btn.clicked() {
                dirties.actions.push(UiAction::NavigateBackFromThread {
                    root_session_id: root_sid,
                });
            }
        });
        ui.add_space(styles.spacing.small);
    }
}

impl BlockRenderCtx<'_> {
    /// Render the inner content of a single block cell (without frame/separator).
    fn render_block_content(
        &mut self,
        ui: &mut egui::Ui,
        cell: &Block,
        styles: &crate::ui::styles::Styles,
        colors: &crate::ui::styles::colors::Colors,
    ) {
        match cell {
            Block::Terminal {
                block_id,
                frozen_output,
            } => {
                let output_text = frozen_output.as_deref().or(self.live_output).unwrap_or("");
                let term_cell_id = 0x1_0000_u32 + *block_id as u32;
                let has_thread = self
                    .session_chats
                    .get(&self.sid)
                    .is_some_and(|c| c.threads.contains_key(&term_cell_id));
                let (f_open, f_text) = self
                    .block_filters
                    .entry(*block_id)
                    .or_insert_with(|| (false, String::new()));
                let mut c = TerminalBlockCtx {
                    sid: self.sid,
                    cell_id: term_cell_id,
                    on_branch: self.pending_branch,
                    has_thread,
                    is_thread: self.is_thread,
                    filter_open: *f_open,
                    filter_text: f_text,
                };
                TerminalBlock::render(
                    ui,
                    *block_id,
                    self.command_blocks,
                    output_text,
                    styles,
                    colors,
                    &mut c,
                );
                self.block_filters.get_mut(block_id).unwrap().0 = c.filter_open;
            }
            Block::Info { id, text } => {
                InfoBlock::render(ui, *id, text, styles, colors);
            }
            Block::Stream {
                id,
                prompt,
                response,
                is_streaming,
                tool_status,
                ..
            } => {
                let tool_calls_log: &mut Vec<ToolCallEntry> =
                    if let Some(pe_blocks_ptr) = self.prompt_editor_blocks {
                        // SAFETY: We hold &mut state which owns prompt_editors.
                        // We only access one specific block element.
                        let pe_blocks = unsafe { &mut *pe_blocks_ptr };
                        if let Some(Block::Stream { tool_calls_log, .. }) =
                            pe_blocks.get_mut(self.cell_idx)
                        {
                            tool_calls_log
                        } else {
                            self.empty_log
                        }
                    } else {
                        self.empty_log
                    };
                let has_thread = self
                    .session_chats
                    .get(&self.sid)
                    .is_some_and(|c| c.threads.contains_key(id));
                let is_starred = self
                    .session_chats
                    .get(&self.sid)
                    .is_some_and(|c| c.starred_ids.contains(id));
                let mut c = StreamBlockCtx {
                    sid: self.sid,
                    cell_id: *id,
                    prompt,
                    response,
                    is_streaming: *is_streaming,
                    tool_status: tool_status.as_deref(),
                    tool_calls_log,
                    session_chats: self.session_chats,
                    starred: is_starred,
                    on_star_toggle: self.pending_star,
                    on_branch: self.pending_branch,
                    has_thread,
                    is_thread: self.is_thread,
                };
                StreamBlock::render(ui, &mut c, styles, colors);
            }
            Block::PinnedContent { text } => {
                egui::Frame::new()
                    .fill(colors.bg_body)
                    .corner_radius(egui::CornerRadius::same(styles.radii.sm as _))
                    .inner_margin(egui::Margin::same(styles.spacing.medium as _))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let cache = self.session_chats.get_mut(&self.sid).map(|c| {
                            c.cell_caches.entry(0).or_default()
                                as &mut egui_commonmark::CommonMarkCache
                        });
                        if let Some(cache) = cache {
                            egui_commonmark::CommonMarkViewer::new()
                                .max_image_width(Some((ui.available_width() * 0.8) as usize))
                                .show(ui, cache, text);
                        } else {
                            ui.label(text);
                        }
                    });
            }
        }
    }
}

// ─── Thread Summary ─────────────────────────────────────────────────────────

/// Render a compact summary of the last N messages in a child thread.
///
/// Shown below the parent stream block that spawned the thread.
fn render_thread_summary(
    ui: &mut egui::Ui,
    thread_sid: SessionId,
    prompt_editors: &std::collections::HashMap<SessionId, PromptState>,
    styles: &crate::ui::styles::Styles,
    colors: &crate::ui::styles::colors::Colors,
) {
    const MAX_PREVIEW: usize = 3;
    let summaries: Vec<(String, String)> = prompt_editors
        .get(&thread_sid)
        .map(|pe| {
            pe.blocks
                .iter()
                .rev()
                .filter_map(|b| {
                    if let Block::Stream {
                        prompt, response, ..
                    } = b
                    {
                        let truncated = if response.len() > 80 {
                            format!("{}…", &response[..80])
                        } else {
                            response.clone()
                        };
                        Some((prompt.clone(), truncated))
                    } else {
                        None
                    }
                })
                .take(MAX_PREVIEW)
                .collect()
        })
        .unwrap_or_default();

    if summaries.is_empty() {
        return;
    }

    let msg_count = prompt_editors
        .get(&thread_sid)
        .map(|pe| {
            pe.blocks
                .iter()
                .filter(|b| matches!(b, Block::Stream { .. }))
                .count()
        })
        .unwrap_or(0);

    ui.add_space(styles.spacing.small);
    egui::Frame::new()
        .fill(colors.bg_body)
        .corner_radius(egui::CornerRadius::same(styles.radii.sm as _))
        .inner_margin(egui::Margin::symmetric(
            styles.spacing.medium as _,
            styles.spacing.small as _,
        ))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(crate::ui::i18n::t_fmt(
                        "chat.thread_summary",
                        &msg_count.to_string(),
                    ))
                    .color(colors.primary)
                    .size(styles.typography.body0),
                );
            });
            ui.add_space(styles.spacing.small);
            for (prompt, response) in summaries.iter().rev() {
                ui.horizontal(|ui| {
                    ui.colored_label(colors.text_caption, "?");
                    ui.label(
                        egui::RichText::new(prompt)
                            .size(styles.typography.body0)
                            .color(colors.text_caption),
                    );
                });
                if !response.is_empty() {
                    ui.label(
                        egui::RichText::new(response)
                            .size(styles.typography.body0)
                            .color(colors.text_caption),
                    );
                }
            }
        });
}
