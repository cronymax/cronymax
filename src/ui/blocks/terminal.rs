//! Terminal (PTY) block widget (§2.3.1) — command prompt + captured output.

use crate::renderer::terminal::SessionId;
use crate::ui::i18n::{t, t_fmt};
use crate::ui::icons;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// Parameters for rendering a terminal block with action buttons.
pub struct TerminalBlockCtx<'a> {
    pub sid: SessionId,
    /// Synthetic cell ID for thread branching (derived from block_id).
    pub cell_id: u32,
    /// Output: pending branch action (session_id, cell_id).
    pub on_branch: &'a mut Option<(u32, u32)>,
    /// Whether this block already has a child thread.
    pub has_thread: bool,
    /// Whether this session is itself a thread.
    pub is_thread: bool,
    /// Whether the inline filter is open for this block.
    pub filter_open: bool,
    /// Filter query text.
    pub filter_text: &'a mut String,
}

/// Terminal block widget — renders a frozen PTY command cell.
pub struct TerminalBlock;

impl TerminalBlock {
    /// Render a frozen terminal command cell (prompt + captured output) using egui.
    ///
    /// `ctx` provides session context for star/branch/filter actions on the
    /// terminal block header.
    pub fn render(
        ui: &mut egui::Ui,
        block_id: usize,
        blocks: &[(String, String)],
        output: &str,
        styles: &Styles,
        colors: &Colors,
        ctx: &mut TerminalBlockCtx<'_>,
    ) -> egui::Response {
        let (prompt_str, cmd_str) = blocks
            .get(block_id)
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .unwrap_or(("", ""));

        ui.set_min_width(ui.available_width());
        let resp_id = ui.id().with("terminal_block").with(block_id);

        // Header row: prompt + command ............... ⑂ filter
        ui.horizontal(|ui| {
            ui.colored_label(colors.primary, prompt_str);
            ui.add(
                egui::Label::new(egui::RichText::new(cmd_str).monospace()).selectable(true),
            );

            // Right-aligned action buttons (visible on hover).
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let row_hovered = ui.ui_contains_pointer();

                // ── Filter toggle ──
                let filter_icon = if ctx.filter_open { "✕" } else { "⌕" };
                let filter_color = if ctx.filter_open {
                    colors.primary
                } else if row_hovered {
                    colors.text_caption
                } else {
                    colors.text_disabled
                };
                let filter_btn = ui.add(
                    egui::Button::new(
                        egui::RichText::new(filter_icon)
                            .color(filter_color)
                            .size(styles.typography.body0),
                    )
                    .frame(false),
                );
                if filter_btn.clicked() {
                    ctx.filter_open = !ctx.filter_open;
                    if !ctx.filter_open {
                        ctx.filter_text.clear();
                    }
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

        // ── Inline filter text input ──
        if ctx.filter_open {
            ui.horizontal(|ui| {
                let te_id = egui::Id::new("term_block_filter")
                    .with(ctx.sid)
                    .with(block_id);
                let resp = ui.add(
                    egui::TextEdit::singleline(ctx.filter_text)
                        .id(te_id)
                        .hint_text("Filter output...")
                        .desired_width(ui.available_width().min(200.0))
                        .font(egui::TextStyle::Small),
                );
                if ctx.filter_text.is_empty() {
                    resp.request_focus();
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    ctx.filter_open = false;
                    ctx.filter_text.clear();
                }
            });
        }

        // ── Output ──
        if !output.is_empty() {
            ui.add_space(styles.spacing.small);
            let filter_q = if ctx.filter_open && !ctx.filter_text.is_empty() {
                Some(ctx.filter_text.to_lowercase())
            } else {
                None
            };
            let max_lines = 40;
            let line_count = output.lines().count();
            let display_text = if let Some(ref q) = filter_q {
                let filtered: Vec<&str> = output
                    .lines()
                    .filter(|l| l.to_lowercase().contains(q))
                    .collect();
                let count = filtered.len();
                if count > max_lines {
                    let lines: Vec<&str> = filtered.into_iter().take(max_lines).collect();
                    let more = t_fmt("chat.more_lines", &(count - max_lines).to_string());
                    format!("{}\n{}", lines.join("\n"), more)
                } else {
                    filtered.join("\n")
                }
            } else if line_count > max_lines {
                let lines: Vec<&str> = output.lines().take(max_lines).collect();
                let more = t_fmt("chat.more_lines", &(line_count - max_lines).to_string());
                format!("{}\n{}", lines.join("\n"), more)
            } else {
                output.to_string()
            };
            let mut text = display_text;
            let te_id = egui::Id::new("term_output").with(ctx.sid).with(block_id);
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .id(te_id)
                    .font(egui::FontId::monospace(styles.typography.body0))
                    .desired_width(ui.available_width())
                    .interactive(true)
                    .frame(false),
            );
        }

        // Invisible response area for context menu.
        ui.interact(ui.min_rect(), resp_id, egui::Sense::hover())
    }
}
