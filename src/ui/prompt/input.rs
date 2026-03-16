//! Keyboard input processing and text synchronization.

use super::*;

/// Bundled context for prompt keyboard input handling.
///
/// Shared between [`PromptEditor::handle_suggestion_keys`] and
/// [`PromptEditor::handle_submit_and_history`].
pub(super) struct PromptInputState<'a> {
    pub ui: &'a mut egui::Ui,
    pub te_response_id: egui::Id,
    pub focused: bool,
    pub lost_focus: bool,
    pub tab_pressed: bool,
    pub sid: SessionId,
    pub submitted: &'a mut Vec<(SessionId, String)>,
}

impl PromptState {
    pub(super) fn detect_prefix_char(&self) -> Option<char> {
        if self.text.starts_with(':') {
            Some(':')
        } else if self.text.starts_with('$') {
            Some('$')
        } else {
            None
        }
    }

    /// Compute the dynamic prompt prefix and the byte offset where display text starts.
    ///
    /// When a `:` command has been selected (e.g. `:webview `), the prompt
    /// displays `:webview` as the prefix and the argument as the editable text.
    /// `$` triggers script mode, and the default (no prefix) is chat mode.
    pub(super) fn compute_dynamic_prompt(
        &self,
        prefix_char: Option<char>,
        commands: &[CommandEntry],
    ) -> (egui::RichText, usize) {
        match prefix_char {
            Some(':') => {
                // Check if the text after `:` matches a known command followed by a space
                let after_colon = &self.text[1..];
                for cmd in commands {
                    let pattern = format!("{} ", cmd.action);
                    if after_colon.starts_with(&pattern) {
                        // Show `:command` as the prompt prefix
                        let prefix = format!(":{} ", cmd.action);
                        let offset = 1 + pattern.len(); // skip `:` + `action `
                        return (egui::RichText::new(prefix).italics(), offset);
                    }
                }
                // No matched command — just show `: ` with offset 1
                (egui::RichText::new(": "), 1)
            }
            Some('$') => (egui::RichText::new("$ "), 1),
            _ => (egui::RichText::new(""), 0), // chat mode (default) — no prefix
        }
    }

    /// In chat mode, detect a bare Enter press (without Shift) and consume all
    /// Enter events so the multiline TextEdit doesn't insert a newline.
    /// Only fires when this pane's TextEdit (`te_id`) had focus last frame,
    /// preventing split panes from stealing each other's Enter events.
    pub(super) fn detect_chat_bare_enter(
        ui: &mut egui::Ui,
        is_chat_mode: bool,
        te_id: egui::Id,
    ) -> bool {
        if !is_chat_mode {
            return false;
        }
        // Only detect Enter for the pane whose TextEdit is focused.
        let te_focused = ui.ctx().memory(|m| m.has_focus(te_id));
        let te_had_focus = ui.ctx().memory(|m| m.had_focus_last_frame(te_id));
        if !te_focused && !te_had_focus {
            return false;
        }
        let bare_enter = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.shift
                )
            })
        });
        if bare_enter {
            ui.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            pressed: true,
                            ..
                        }
                    )
                });
            });
        }
        bare_enter
    }

    /// Sync the display text (without prefix) back into `self.text` (with prefix).
    /// Handles backspace-on-empty to revert to default `>` prompt mode.
    pub(super) fn sync_display_text(
        &mut self,
        ui: &mut egui::Ui,
        prefix_char: Option<char>,
        display_text_start: usize,
        display_text: String,
    ) {
        match prefix_char {
            Some(_) => {
                let backspace_on_empty =
                    display_text.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Backspace));
                if backspace_on_empty {
                    // If we have a long prefix (e.g. `:webview `), backspace should
                    // first shrink back to just `:`. Only clear fully if already at `:`.
                    if display_text_start > 1 {
                        // Go back to just `:` with the command name minus last char
                        self.text = ":".to_string();
                    } else {
                        self.text.clear();
                    }
                } else {
                    // Reconstruct: the prompt_prefix (without trailing space) has already been shown,
                    // but we need to rebuild self.text = prefix_part + display_text.
                    let prefix_part = &self.text[..display_text_start];
                    self.text = format!("{}{}", prefix_part, display_text);
                }
            }
            None => {
                self.text = display_text;
            }
        }
    }

    /// Handle keyboard events when the suggestion panel is showing:
    /// Escape to dismiss, Up/Down to navigate, Enter/Tab to select.
    pub(super) fn handle_suggestion_keys(
        &mut self,
        ctx: &mut PromptInputState<'_>,
        te_id: egui::Id,
        filtered_cmds: &[(usize, &CommandEntry)],
    ) {
        let filter_cmds_len = filtered_cmds.len();

        if ctx.focused && ctx.ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.text.clear();
            self.cmd_suggestion_idx = 0;
        }
        if ctx.focused && ctx.ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.cmd_suggestion_idx =
                (self.cmd_suggestion_idx + filter_cmds_len - 1) % filter_cmds_len;
        }
        if ctx.focused && ctx.ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.cmd_suggestion_idx = (self.cmd_suggestion_idx + 1) % filter_cmds_len;
        }

        if ctx.lost_focus && ctx.ui.input(|i| i.key_pressed(egui::Key::Enter)) || ctx.tab_pressed {
            if let Some((_orig, cmd)) = filtered_cmds.get(self.cmd_suggestion_idx) {
                if cmd.needs_arg {
                    self.text = format!(":{} ", cmd.action);
                    if let Some(mut te_state) = egui::TextEdit::load_state(ctx.ui.ctx(), te_id) {
                        let end = egui::text::CCursor::new(self.text.len());
                        #[allow(deprecated)]
                        te_state.set_ccursor_range(Some(egui::text::CCursorRange::one(end)));
                        te_state.store(ctx.ui.ctx(), te_id);
                    }
                } else {
                    let cmd_text = format!(":{}", cmd.action);
                    self.text.clear();
                    self.cmd_suggestion_idx = 0;
                    ctx.submitted.push((ctx.sid, cmd_text));
                }
            }
            ctx.ui
                .ctx()
                .memory_mut(|mem| mem.request_focus(ctx.te_response_id));
            ctx.ui.ctx().request_repaint();
        }
    }

    /// Handle keyboard events in normal (non-suggestion) mode:
    /// Enter to submit, Up/Down for history, Tab for path completion.
    pub(super) fn handle_submit_and_history(
        &mut self,
        ctx: &mut PromptInputState<'_>,
        is_chat_mode: bool,
        chat_bare_enter: bool,
    ) {
        let should_submit = if is_chat_mode {
            chat_bare_enter
        } else {
            ctx.lost_focus && ctx.ui.input(|i| i.key_pressed(egui::Key::Enter))
        };
        if should_submit {
            if let Some(cmd) = self.submit() {
                ctx.submitted.push((ctx.sid, cmd));
            }
            ctx.ui
                .ctx()
                .memory_mut(|mem| mem.request_focus(ctx.te_response_id));
            ctx.ui.ctx().request_repaint();
        }

        self.cmd_suggestion_idx = 0;

        if ctx.focused && !is_chat_mode {
            if ctx.ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                self.history_navigate(-1);
            }
            if ctx.ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                self.history_navigate(1);
            }
            // Tab: path auto-completion (Tab event already consumed before TextEdit).
            if ctx.tab_pressed {
                let (new_text, _result) = crate::ui::completion::complete_path(&self.text);
                if new_text != self.text {
                    self.text = new_text;
                }
                ctx.ui.ctx().request_repaint();
            }
        }
    }
}
