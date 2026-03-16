use std::rc::Rc;

use crate::{
    terminal::SessionId,
    ui::{
        CommandEntry,
        i18n::t,
        prompt::PromptState,
        styles::{Styles, colors::Colors},
        tiles::TerminalLayout,
        widget::{Fragment, Widget},
    },
};

/// Inner prompt editor: suggestion panel, context bar, text edit, hint bar,
/// keyboard handling.
pub(super) struct PromptEditorWidget<'a> {
    pub state: &'a mut PromptState,
    pub sid: SessionId,
    pub layout: TerminalLayout,
    pub commands: &'a [CommandEntry],
    pub filtered_commands: &'a [(usize, &'a CommandEntry)],
    pub is_chat_mode: bool,
    pub address_bar_editing: bool,
    pub show_suggestions: bool,
}

impl Widget<egui::Ui> for PromptEditorWidget<'_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let styles = f.styles;
        let colors = Rc::clone(&f.colors);
        let sid = self.sid;
        let layout = self.layout;
        let is_chat_mode = self.is_chat_mode;
        let commands = self.commands;

        // ── Context bar (always visible when input is shown) ──
        let ui = &mut *f.painter;
        self.draw_context_bar(ui, layout, styles, &colors, is_chat_mode, sid);

        // ── Row 1: prompt prefix + text editor (horizontal) ──
        let te_id = egui::Id::new("pane_input_te").with(sid);
        let chat_bare_enter = PromptState::detect_chat_bare_enter(ui, is_chat_mode, te_id);
        let tab_pressed: bool = ui.ctx().data(|d| {
            d.get_temp(egui::Id::new("__global_tab_pressed"))
                .unwrap_or(false)
        });

        // Dynamic prompt prefix.
        let prefix_char = self.state.detect_prefix_char();
        let (dynamic_prompt, display_text_start) =
            self.state.compute_dynamic_prompt(prefix_char, commands);

        // Display text without prefix to avoid duplication.
        let mut display_text = if display_text_start < self.state.text.len() {
            self.state.text[display_text_start..].to_string()
        } else if prefix_char.is_some() && display_text_start > 0 {
            String::new()
        } else {
            self.state.text.clone()
        };

        let te_response = ui
            .horizontal(|ui| {
                // Give the prefix the same vertical margin as the TextEdit
                // so that text baselines align.
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(0, styles.spacing.large as _))
                    .show(ui, |ui| {
                        ui.colored_label(colors.primary, dynamic_prompt);
                    });

                // Create and render the TextEdit widget.
                let text_edit = if is_chat_mode {
                    // Compute actual line count for proper sizing.
                    let n_lines = display_text.split('\n').count().clamp(1, 10);
                    egui::TextEdit::multiline(&mut display_text).desired_rows(n_lines)
                } else {
                    egui::TextEdit::singleline(&mut display_text)
                };
                ui.add(
                    text_edit
                        .id(te_id)
                        .font(egui::TextStyle::Monospace)
                        .text_color(colors.text_title)
                        .desired_width(ui.available_width())
                        .hint_text(
                            egui::RichText::new(t("prompt.input.placeholder"))
                                .color(colors.text_placeholder),
                        )
                        .margin(egui::Margin::symmetric(
                            styles.spacing.small as _,
                            styles.spacing.large as _,
                        ))
                        .frame(false),
                )
            })
            .inner;

        // ── Hint bar ──
        Self::draw_hint_bar(ui, layout, styles, &colors);

        // Sync display text back (with prefix).
        self.state
            .sync_display_text(ui, prefix_char, display_text_start, display_text);

        // ── File picker (#-trigger) ───────────────────────
        self.state
            .update_file_picker_state(ui, te_id, display_text_start);

        // Focus state.
        let focused = ui.ctx().memory(|m| m.focused()) == Some(te_response.id);
        let had_focus = ui.ctx().memory(|m| m.had_focus_last_frame(te_response.id));
        let lost_focus = had_focus && !focused;

        // Delegate keyboard handling.
        let submitted = &mut f.dirties.commands;
        if self.state.file_picker.active {
            self.state
                .handle_file_picker_keys(ui, te_response.id, focused, sid, submitted);
        } else if self.show_suggestions {
            let mut state = super::input::PromptInputState {
                ui: &mut *ui,
                te_response_id: te_response.id,
                focused,
                lost_focus,
                tab_pressed,
                sid,
                submitted: &mut *submitted,
            };
            self.state
                .handle_suggestion_keys(&mut state, te_id, self.filtered_commands);
        } else {
            let mut state = super::input::PromptInputState {
                ui: &mut *ui,
                te_response_id: te_response.id,
                focused,
                lost_focus,
                tab_pressed,
                sid,
                submitted: &mut *submitted,
            };
            self.state
                .handle_submit_and_history(&mut state, is_chat_mode, chat_bare_enter);
        }

        // Pick up files selected via the native dialog (from background thread).
        let pending: Option<String> = ui
            .ctx()
            .data(|d| d.get_temp(egui::Id::new("__pending_context_files")));
        if let Some(files) = pending {
            self.state.text.push_str(&files);
            ui.ctx().data_mut(|d| {
                d.remove_temp::<String>(egui::Id::new("__pending_context_files"));
            });
        }

        // Auto-focus the input when nothing else wants keyboard.
        if !focused && !self.address_bar_editing && !ui.ctx().wants_keyboard_input() {
            te_response.request_focus();
        }
    }
}

impl PromptEditorWidget<'_> {
    /// Render the context bar above the prompt editor.
    /// Shows model selector first, then context window indicator in chat mode.
    fn draw_context_bar(
        &mut self,
        ui: &mut egui::Ui,
        layout: TerminalLayout,
        styles: &Styles,
        colors: &Colors,
        is_chat_mode: bool,
        sid: SessionId,
    ) {
        let dim = colors.text_caption;
        let small = egui::FontId::proportional(styles.typography.caption1);

        ui.horizontal(|ui| {
            if is_chat_mode {
                // ── Model selector (egui ComboBox) — first item ──
                if !self.state.model_items.is_empty() {
                    // Clamp index to valid range.
                    if self.state.selected_model_idx >= self.state.model_items.len() {
                        self.state.selected_model_idx = 0;
                    }
                    let prev_idx = self.state.selected_model_idx;
                    let current_label = self.state.model_items[self.state.selected_model_idx]
                        .display_label
                        .clone();

                    egui::ComboBox::from_id_salt(egui::Id::new("model_selector").with(sid))
                        .selected_text(
                            egui::RichText::new(&current_label).size(styles.typography.body0),
                        )
                        .show_ui(ui, |ui| {
                            for (i, item) in self.state.model_items.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.state.selected_model_idx,
                                    i,
                                    &item.display_label,
                                );
                            }
                        });
                    if self.state.selected_model_idx != prev_idx {
                        self.state.last_model_selection =
                            Some(self.state.model_items[self.state.selected_model_idx].clone());
                    }

                    ui.add_space(styles.spacing.medium);
                    ui.label(egui::RichText::new("·").font(small.clone()).color(dim));
                    ui.add_space(styles.spacing.medium);
                }

                // Context window indicator.
                if let Some((used, limit)) = layout.chat_context {
                    let pct = if limit > 0 {
                        (used as f32 / limit as f32 * 100.0).min(100.0)
                    } else {
                        0.0
                    };
                    let color = if pct > 90.0 {
                        colors.warning // warning color
                    } else {
                        dim
                    };
                    let label = crate::ui::i18n::t_fmt(
                        "prompt.context.pct_used",
                        &format!("{}", pct as u32),
                    );
                    ui.label(egui::RichText::new(label).font(small.clone()).color(color));
                } else {
                    ui.label(
                        egui::RichText::new(t("prompt.context.window"))
                            .font(small.clone())
                            .color(dim),
                    );
                }
            } else {
                // Non-chat mode: show a subtle mode indicator.
                ui.label(
                    egui::RichText::new(t("prompt.mode.terminal"))
                        .font(small.clone())
                        .color(dim),
                );
            }
        });
    }

    /// Render the hint bar below the prompt editor showing available mode prefixes.
    fn draw_hint_bar(ui: &mut egui::Ui, layout: TerminalLayout, styles: &Styles, colors: &Colors) {
        let TerminalLayout { is_chat_mode, .. } = layout;
        ui.horizontal(|ui| {
            let dim = colors.text_caption;
            let small = egui::FontId::proportional(styles.typography.caption1);
            let hints: Vec<(&str, &str)> = if is_chat_mode {
                vec![
                    (":", t("hint.command")),
                    ("$", t("hint.script")),
                    ("  enter", t("hint.send")),
                    ("  shift+enter", t("hint.newline")),
                ]
            } else {
                vec![
                    (":", t("hint.command")),
                    ("$", t("hint.script")),
                    ("  tab", t("hint.complete")),
                    ("  enter", t("hint.submit")),
                ]
            };
            for (key, desc) in &hints {
                ui.label(
                    egui::RichText::new(*key)
                        .font(small.clone())
                        .italics()
                        .color(colors.primary),
                );
                ui.label(
                    egui::RichText::new(*desc)
                        .font(small.clone())
                        .italics()
                        .color(dim),
                );
            }
        });
    }
}
