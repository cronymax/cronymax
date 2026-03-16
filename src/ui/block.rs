//! Block types and renderers — content units displayed inside a pane (§2.3).
//!
//! Each pane contains an ordered list of [`BlockMode`] entries rendered in
//! chronological order:
//! - **Terminal block** (§2.3.1): PTY command + captured output
//! - **Stream block** (§2.3.2): SSE/LLM exchange with streaming, tool calls,
//!   and incremental markdown rendering

use std::rc::Rc;

use crate::renderer::terminal::SessionId;
use crate::ui::i18n::{t, t_fmt};
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;
use crate::ui::widget::{Fragment, Widget};

// ─── Block Mode ─────────────────────────────────────────────────────────────

/// A content block in the unified pane cell list — see widget hierarchy §2.3.
///
/// - [`BlockMode::Terminal`] (§2.3.1): PTY command + output
/// - [`BlockMode::Stream`] (§2.3.2): SSE/LLM exchange
#[derive(Debug, Clone)]
pub enum BlockMode {
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
pub struct Block {
    /// Logical height of one terminal text row in pixels.
    pub block_height: f32,
    /// Number of scrollback history lines currently stored.
    pub history_size: usize,
    /// Number of visible screen lines (pane's current row count).
    pub screen_lines: usize,
    /// Lines scrolled above the bottom of the scrollback (0 = not scrolled).
    pub display_offset: usize,
}

// ─── Block Widget ───────────────────────────────────────────────────────────

/// Block inline widget — renders terminal or stream blocks inside a pane.
pub struct BlockWidget<'a> {
    /// The block mode to render.
    pub mode: &'a mut BlockMode,
    /// Command blocks for terminal mode (prompt, command pairs).
    pub command_blocks: &'a [(String, String)],
    /// Session ID for stream block star toggle.
    pub session_id: SessionId,
    /// Session chat state for render_stream_block.
    pub session_chats: &'a mut std::collections::HashMap<SessionId, crate::ui::chat::SessionChat>,
    /// Pending star toggle output.
    pub on_star_toggle: &'a mut Option<(u32, u32)>,
}

// ─── Block Renderers (associated functions on BlockWidget) ──────────────────

/// Parameters for rendering a stream block.
pub struct StreamBlockCtx<'a> {
    pub sid: SessionId,
    pub cell_id: u32,
    pub prompt: &'a str,
    pub response: &'a str,
    pub is_streaming: bool,
    pub tool_status: Option<&'a str>,
    pub tool_calls_log: &'a mut [ToolCallEntry],
    pub session_chats: &'a mut std::collections::HashMap<SessionId, crate::ui::chat::SessionChat>,
    pub starred: bool,
    pub on_star_toggle: &'a mut Option<(u32, u32)>,
}

impl BlockWidget<'_> {
    /// Render a frozen terminal command cell (prompt + captured output) using egui.
    pub fn render_terminal_block(
        ui: &mut egui::Ui,
        block_id: usize,
        blocks: &[(String, String)],
        output: &str,
        styles: &Styles,
        colors: &Colors,
    ) -> egui::Response {
        let (prompt_str, cmd_str) = blocks
            .get(block_id)
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .unwrap_or(("", ""));

        ui.set_min_width(ui.available_width());
        let resp_id = ui.id().with("terminal_block").with(block_id);

        ui.horizontal(|ui| {
            ui.colored_label(colors.primary, prompt_str);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(cmd_str).monospace(), // .color(ui.visuals().text_color()),
                )
                .selectable(true),
            );
        });

        if !output.is_empty() {
            ui.add_space(styles.spacing.small);
            // Truncate very long outputs for performance.
            let max_lines = 40;
            let line_count = output.lines().count();
            let display_text = if line_count > max_lines {
                let lines: Vec<&str> = output.lines().take(max_lines).collect();
                let more = t_fmt("chat.more_lines", &(line_count - max_lines).to_string());
                format!("{}\n{}", lines.join("\n"), more)
            } else {
                output.to_string()
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(display_text)
                        .monospace()
                        .size(styles.typography.body0),
                )
                .wrap()
                .selectable(true),
            );
        }

        // Invisible response area for context menu (hover-only so hyperlinks
        // inside the block can still receive primary-click events).
        ui.interact(ui.min_rect(), resp_id, egui::Sense::hover())
    }

    /// Render an informational message block (italic, muted, no prompt marker or star).
    pub fn render_info_block(
        ui: &mut egui::Ui,
        _cell_id: u32,
        text: &str,
        styles: &Styles,
        _colors: &Colors,
    ) -> egui::Response {
        ui.set_min_width(ui.available_width());
        let resp_id = ui.id().with("info_block").with(_cell_id);
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .italics()
                    .size(styles.typography.body0),
            )
            .wrap()
            .selectable(true),
        );
        ui.interact(ui.min_rect(), resp_id, egui::Sense::hover())
    }

    pub fn render_stream_block(
        ui: &mut egui::Ui,
        ctx: &mut StreamBlockCtx<'_>,
        styles: &Styles,
        colors: &Colors,
    ) -> egui::Response {
        ui.set_min_width(ui.available_width());
        let resp_id = ui.id().with("stream_block").with(ctx.cell_id);

        // Header: ? prompt ..................... ★/☆
        ui.horizontal(|ui| {
            ui.colored_label(colors.primary, "?");
            ui.strong(ctx.prompt);

            // Right-aligned star toggle button.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let star_label = if ctx.starred { "★" } else { "☆" };
                let btn = ui.add(
                    egui::Button::new(
                        egui::RichText::new(star_label)
                            .color(if ctx.starred {
                                colors.primary
                            } else {
                                egui::Color32::TRANSPARENT
                            })
                            .size(styles.typography.headline),
                    )
                    .frame(false),
                );
                if btn.clicked() {
                    *ctx.on_star_toggle = Some((ctx.sid, ctx.cell_id));
                }
                if btn.hovered() {
                    btn.on_hover_text(if ctx.starred {
                        "Unstar this block (won't be compacted)"
                    } else {
                        "Star this block (preserved during compaction)"
                    });
                }
            });
        });

        // Show tool invocation status when the LLM is calling tools.
        if let Some(tool_name) = ctx.tool_status {
            ui.add_space(styles.spacing.small);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.colored_label(colors.primary, t_fmt("chat.tool_calling", tool_name));
            });
        }

        // Show tool call log entries (collapsed by default).
        if !ctx.tool_calls_log.is_empty() {
            ui.add_space(styles.spacing.small);
            for entry in ctx.tool_calls_log.iter_mut() {
                Self::render_tool_call_entry(ui, entry, styles, colors);
            }
        }

        if !ctx.response.is_empty() {
            ui.add_space(styles.spacing.small);
            if let Some(chat) = ctx.session_chats.get_mut(&ctx.sid) {
                let cache = chat
                    .cell_caches
                    .entry(ctx.cell_id)
                    .or_insert_with(egui_commonmark::CommonMarkCache::default);
                if ctx.is_streaming {
                    crate::ai::markdown::render_streaming(ui, ctx.response, cache);
                } else {
                    crate::ai::markdown::render_message(ui, ctx.response, cache);
                }
            } else {
                ui.label(ctx.response);
            }
        }

        if ctx.is_streaming && ctx.response.is_empty() && ctx.tool_status.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(t("chat.thinking"));
            });
        } else if ctx.is_streaming && ctx.tool_status.is_none() {
            ui.label("▌");
        }

        // Invisible response area for context menu (hover-only so hyperlinks
        // inside the block can still receive primary-click events).
        ui.interact(ui.min_rect(), resp_id, egui::Sense::hover())
    }

    /// Render a single tool call log entry as a collapsible header.
    ///
    /// The frame's response is upgraded to `Sense::click()` to toggle
    /// `entry.expanded`.  Selectable labels in the expanded body have smaller
    /// rects and therefore win click hit-testing, so text selection still works.
    fn render_tool_call_entry(
        ui: &mut egui::Ui,
        entry: &mut ToolCallEntry,
        styles: &Styles,
        colors: &Colors,
    ) {
        let status_color = if entry.in_progress {
            colors.info
        } else {
            colors.primary
        };

        // Themed background frame for the tool call entry.
        // let bg = colors.terminal_border;
        egui::Frame::new()
            .fill(colors.bg_body)
            .corner_radius(egui::CornerRadius::same(styles.radii.sm as _))
            .inner_margin(egui::Margin::symmetric(
                styles.spacing.medium as _,
                styles.spacing.small as _,
            ))
            .show(ui, |ui| {
                ui.collapsing(
                    egui::RichText::new(&entry.name)
                        .monospace()
                        .color(status_color)
                        .size(styles.typography.body0),
                    |ui| {
                        if !entry.summary.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&entry.summary)
                                        .monospace()
                                        .italics()
                                        .size(styles.typography.body0),
                                )
                                .selectable(true),
                            );
                        }

                        if let Some(result) = &entry.result {
                            ui.add_space(styles.spacing.small);
                            // Truncate long results
                            let max_chars = 2000;
                            let display = if result.len() > max_chars {
                                format!("{}...", &result[..max_chars])
                            } else {
                                result.clone()
                            };
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(display)
                                        .monospace()
                                        .size(styles.typography.body0),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        }
                    },
                );
            });

        // // Upgrade the frame's hover-only response to accept clicks.
        // // This registers the click handler in the parent UI fragment where it
        // // properly participates in egui's hit-testing (smaller rect than the
        // // stream block's full hover overlay → wins click resolution).
        // if resp.response.interact(egui::Sense::click()).clicked() {
        //     entry.expanded = !entry.expanded;
        // }
    }

    /// Collect the copyable text content from a block.
    pub fn block_text_content(
        mode: &BlockMode,
        blocks: &[(String, String)],
        live_output: Option<&str>,
    ) -> String {
        match mode {
            BlockMode::Terminal {
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
            BlockMode::Info { text, .. } => text.clone(),
            BlockMode::Stream {
                prompt, response, ..
            } => {
                if response.is_empty() {
                    format!("? {}", prompt)
                } else {
                    format!("? {}\n{}", prompt, response)
                }
            }
        }
    }
}

impl Widget<egui::Ui> for BlockWidget<'_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let styles = f.styles;
        let colors_rc = Rc::clone(&f.colors);
        let ui = f.ui();
        let colors = &*colors_rc;
        match self.mode {
            BlockMode::Terminal {
                block_id,
                frozen_output,
            } => {
                let output = frozen_output.as_deref().unwrap_or("");
                Self::render_terminal_block(
                    ui,
                    *block_id,
                    self.command_blocks,
                    output,
                    styles,
                    colors,
                );
            }
            BlockMode::Info { id, text } => {
                Self::render_info_block(ui, *id, text, styles, colors);
            }
            BlockMode::Stream {
                id,
                prompt,
                response,
                is_streaming,
                tool_status,
                tool_calls_log,
            } => {
                let mut c = StreamBlockCtx {
                    sid: self.session_id,
                    cell_id: *id,
                    prompt,
                    response,
                    is_streaming: *is_streaming,
                    tool_status: tool_status.as_deref(),
                    tool_calls_log,
                    session_chats: self.session_chats,
                    starred: false, // starred state is managed externally
                    on_star_toggle: self.on_star_toggle,
                };
                Self::render_stream_block(ui, &mut c, styles, colors);
            }
        }
    }
}
