//! Stream (SSE/LLM) block widget (§2.3.2) — prompt + response with markdown.

use crate::renderer::terminal::SessionId;
use crate::ui::chat::SessionChat;
use crate::ui::i18n::{t, t_fmt};
use crate::ui::icons;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

use super::ToolCallEntry;

/// Parameters for rendering a stream block.
pub struct StreamBlockCtx<'a> {
    pub sid: SessionId,
    pub cell_id: u32,
    pub prompt: &'a str,
    pub response: &'a str,
    pub is_streaming: bool,
    pub tool_status: Option<&'a str>,
    pub tool_calls_log: &'a mut [ToolCallEntry],
    pub session_chats: &'a mut std::collections::HashMap<SessionId, SessionChat>,
    pub starred: bool,
    pub on_star_toggle: &'a mut Option<(u32, u32)>,
    /// Output: pending branch action (session_id, cell_id).
    pub on_branch: &'a mut Option<(u32, u32)>,
    /// Whether this block already has a child thread.
    pub has_thread: bool,
    /// Whether this session is itself a thread (hides branch button).
    pub is_thread: bool,
}

/// Stream block widget — renders an LLM exchange (prompt + streamed response).
pub struct StreamBlock;

impl StreamBlock {
    pub fn render(
        ui: &mut egui::Ui,
        ctx: &mut StreamBlockCtx<'_>,
        styles: &Styles,
        colors: &Colors,
    ) -> egui::Response {
        ui.set_min_width(ui.available_width());
        let resp_id = ui.id().with("stream_block").with(ctx.cell_id);

        // Header: ? prompt ..................... ⑂ ★/☆
        let _header_resp = ui.horizontal(|ui| {
            ui.colored_label(colors.primary, "?");
            ui.strong(ctx.prompt);

            // Right-aligned action buttons — visible on hover of the header row.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let row_hovered = ui.ui_contains_pointer();

                // ── Star toggle ──
                let star_label = if ctx.starred { "★" } else { "☆" };
                let star_color = if ctx.starred {
                    colors.primary
                } else if row_hovered {
                    colors.text_caption
                } else {
                    colors.text_disabled
                };
                let btn = ui.add(
                    egui::Button::new(
                        egui::RichText::new(star_label)
                            .color(star_color)
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

                // ── Branch / thread button ──
                if !ctx.is_thread {
                    let branch_color = if ctx.has_thread {
                        colors.primary
                    } else if row_hovered {
                        colors.text_caption
                    } else {
                        colors.text_disabled
                    };
                    let branch_btn = icons::icon_button(
                        ui,
                        icons::IconButtonCfg {
                            icon: icons::Icon::GitPullRequestCreate,
                            tooltip: if ctx.has_thread {
                                t("chat.view_thread")
                            } else {
                                t("chat.start_thread")
                            },
                            base_color: branch_color,
                            hover_color: colors.primary,
                            pixel_size: styles.typography.headline,
                            margin: 2.0,
                        },
                    );
                    if branch_btn.clicked() {
                        *ctx.on_branch = Some((ctx.sid, ctx.cell_id));
                    }
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
                render_tool_call_entry(ui, entry, styles, colors);
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

        // Invisible response area for context menu.
        ui.interact(ui.min_rect(), resp_id, egui::Sense::hover())
    }
}

/// Render a single tool call log entry as a collapsible header.
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
}
