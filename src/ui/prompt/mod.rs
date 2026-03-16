//! Prompt widget (§2.4) — Warp-style command input with history.
//!
//! Comprises:
//! - **Suggestion panel**: commands / file picker
//! - **Prompt editor**: context bar, text edit, hint bar

mod editor;
mod input;

mod cmd_picker;
mod file_picker;

use std::rc::Rc;

use crate::ai::client::ModelListItem;
use crate::ui::block::BlockMode;
use crate::ui::widget::Widget;

use crate::renderer::terminal::SessionId;
use crate::ui::file_picker::FilePickerState;
use crate::ui::tiles::TerminalLayout;
use crate::ui::types::CommandEntry;

/// Fuzzy-match `query` against `target`. Returns a positive score if all query
/// characters appear in order within the target, or 0 for no match.
///
/// Scoring rewards:
/// - Exact substring match (highest bonus)
/// - Consecutive character matches (run bonus)
/// - Matches at word boundaries (after space, `-`, `_`)
/// - Matches at the start of the string
fn fuzzy_score(query: &str, target: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }
    // Exact substring gets top score.
    if target.contains(query) {
        return 1000 + (100 - target.len() as i32).max(0);
    }

    let query_chars: Vec<char> = query.chars().collect();
    let target_chars: Vec<char> = target.chars().collect();

    let mut qi = 0;
    let mut score: i32 = 0;
    let mut consecutive = 0;
    let mut first_match_pos = None;

    for (ti, &tc) in target_chars.iter().enumerate() {
        if qi < query_chars.len() && tc == query_chars[qi] {
            if first_match_pos.is_none() {
                first_match_pos = Some(ti);
            }
            qi += 1;
            consecutive += 1;
            // Consecutive run bonus (accelerating).
            score += consecutive * 2;
            // Word boundary bonus: match right after separator or at start.
            if ti == 0 || matches!(target_chars.get(ti.wrapping_sub(1)), Some(' ' | '-' | '_')) {
                score += 5;
            }
        } else {
            consecutive = 0;
        }
    }

    if qi < query_chars.len() {
        return 0; // Not all query chars matched.
    }

    // Bonus for matching near the start.
    if let Some(pos) = first_match_pos {
        score += (10 - pos as i32).max(0);
    }

    score.max(1)
}
// TerminalMode has been removed — Chat mode is now the only mode.
// The Classic (raw PTY) mode is no longer supported.

/// A recorded command issued in Chat mode.  Used to draw egui cell overlays
/// around each prompt+output block in the terminal pane.
#[derive(Debug, Clone)]
pub struct CommandBlock {
    /// Stable index (== position in `InputLineState::command_blocks`).
    #[allow(dead_code)]
    pub id: usize,
    /// Prompt text shown when the command was typed.
    pub prompt: String,
    /// The command text submitted by the user.
    pub cmd: String,
    /// Absolute row at the time of submission.
    /// `abs_row = history_size + cursor_viewport_line` (both from TermState).
    /// Used each frame to derive the current viewport row.
    pub abs_row: i32,
    /// Current filter pattern for this cell.  Empty ⇒ no filter.
    pub filter_text: String,
    /// Whether the filter input box is currently expanded.
    pub filter_open: bool,
}

/// Per-session state for the editor-mode input line.
#[derive(Debug, Clone)]
pub struct PromptState {
    /// Current text in the input buffer.
    pub text: String,
    /// Shell prompt prefix string (e.g. "$ " or "> ").
    pub prefix: String,
    /// Command history (oldest first).
    pub history: Vec<String>,
    /// Current position in history for Up/Down navigation. None = not browsing.
    pub history_index: Option<usize>,
    /// Saved text when user starts browsing history.
    saved_text: String,
    /// Whether the input line should be rendered.
    pub visible: bool,
    /// Maximum history entries to keep.
    pub max_history: usize,
    /// Recorded command blocks (editor mode). Each entry corresponds to one
    /// submitted command and is used to draw the cell overlay.
    pub command_blocks: Vec<CommandBlock>,
    /// Ordered cell list (editor mode). Each entry corresponds to one
    /// user submission — either a terminal command or a chat exchange.
    pub blocks: Vec<BlockMode>,
    /// Next unique ID for chat cells (monotonically increasing).
    pub next_chat_cell_id: u32,
    /// Selected index in the inline command suggestion list (when `:` prefix active).
    pub cmd_suggestion_idx: usize,
    /// File picker state for `#` trigger.
    pub file_picker: FilePickerState,
    /// Current working directory of the session's shell (updated each frame).
    pub cwd: Option<String>,
    /// Index of the currently selected model in `model_items`.
    pub selected_model_idx: usize,
    /// Available models for the dropdown selector.
    pub model_items: Vec<ModelListItem>,
    /// Last model selection (consumed by caller after draw).
    pub last_model_selection: Option<ModelListItem>,
}

impl PromptState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            prefix: "> ".into(),
            history: Vec::new(),
            history_index: None,
            saved_text: String::new(),
            visible: true,
            max_history: 500,
            command_blocks: Vec::new(),
            blocks: Vec::new(),
            next_chat_cell_id: 0,
            cmd_suggestion_idx: 0,
            file_picker: FilePickerState::new(),
            cwd: None,
            selected_model_idx: 0,
            model_items: Vec::new(),
            last_model_selection: None,
        }
    }

    /// Submit the current text as a command. Returns the command if non-empty.
    /// Pushes to history (deduplicates consecutive), resets state.
    pub fn submit(&mut self) -> Option<String> {
        let cmd = self.text.trim().to_string();
        if cmd.is_empty() {
            return None;
        }

        // Deduplicate: don't push if same as last entry
        if self.history.last() != Some(&cmd) {
            self.history.push(cmd.clone());
            if self.history.len() > self.max_history {
                self.history.remove(0);
            }
        }

        self.text.clear();
        self.history_index = None;
        self.saved_text.clear();
        Some(cmd)
    }

    /// Navigate history: direction = -1 for Up (older), +1 for Down (newer).
    pub fn history_navigate(&mut self, direction: i32) {
        if self.history.is_empty() {
            return;
        }

        match self.history_index {
            None => {
                if direction < 0 {
                    // Start browsing from most recent
                    self.saved_text = self.text.clone();
                    let idx = self.history.len() - 1;
                    self.history_index = Some(idx);
                    self.text = self.history[idx].clone();
                }
                // Down when not browsing → no-op
            }
            Some(idx) => {
                if direction < 0 {
                    // Go older
                    if idx > 0 {
                        let new_idx = idx - 1;
                        self.history_index = Some(new_idx);
                        self.text = self.history[new_idx].clone();
                    }
                    // Already at oldest → no-op
                } else {
                    // Go newer
                    if idx + 1 < self.history.len() {
                        let new_idx = idx + 1;
                        self.history_index = Some(new_idx);
                        self.text = self.history[new_idx].clone();
                    } else {
                        // Past newest → restore saved text
                        self.history_index = None;
                        self.text = self.saved_text.clone();
                        self.saved_text.clear();
                    }
                }
            }
        }
    }
}

impl Default for PromptState {
    fn default() -> Self {
        Self::new()
    }
}

/// Outer prompt widget: Frame wrapper with cursor style, margins, and rounding.
/// Delegates to [`PromptEditorWidget`] for the inner content.
pub struct PromptWidget<'a> {
    pub state: &'a mut PromptState,
    pub sid: SessionId,
    pub layout: TerminalLayout,
    pub commands: &'a [CommandEntry],
    pub address_bar_editing: bool,
}

impl Widget<egui::Ui> for PromptWidget<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Ui as crate::ui::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: crate::ui::widget::Context<'a>,
    ) {
        let styles = ctx.styles;
        let colors = Rc::clone(&ctx.colors);
        let is_chat_mode = self.layout.is_chat_mode;
        let margin = styles.spacing.large;

        // Shrink input_rect by margin for inner layout positioning.
        let mut prompt_layout = self.layout;
        prompt_layout.input_rect = self.layout.input_rect.shrink(margin);

        // Cursor style — use prompt-specific colour, disable blink.
        ui.visuals_mut().text_cursor.stroke =
            egui::Stroke::new(styles.sizes.border * 2.0, colors.primary);
        ui.visuals_mut().text_cursor.blink = false;

        // ── Compute and render inline suggestions ──
        let (show_command_suggestions, filtered_commands) =
            Self::compute_command_suggestions(&self.state.text, self.commands);

        let show_file_picker =
            self.state.file_picker.active && self.state.file_picker.matches_count() > 0;
        // Consume file pick from click
        if let Some(path) = self.state.file_picker.picked_path.take() {
            self.state.insert_file_pick(&path);
        }

        // Render prompt frame inline — height is determined by content.
        egui::Frame::new()
            .fill(colors.bg_float)
            .outer_margin(egui::Margin::same(margin as i8))
            .corner_radius(egui::CornerRadius {
                se: styles.radii.md as u8,
                sw: styles.radii.md as u8,
                ne: if show_command_suggestions {
                    styles.radii.sm as u8
                } else {
                    styles.radii.md as u8
                },
                nw: if show_command_suggestions {
                    styles.radii.sm as u8
                } else {
                    styles.radii.md as u8
                },
            })
            .show(ui, |ui| {
                if show_command_suggestions || show_file_picker {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(styles.spacing.medium as i8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());

                            if show_command_suggestions {
                                ctx.bind::<egui::Ui>(ui)
                                    .add(cmd_picker::CommandPickerWidget {
                                        state: &mut *self.state,
                                        filtered_items: &filtered_commands,
                                        sid: self.sid,
                                    });
                            }

                            // Render file picker popup widget
                            if self.state.file_picker.active
                                && self.state.file_picker.matches_count() > 0
                            {
                                ctx.bind::<egui::Ui>(ui).add(file_picker::FilePickerWidget {
                                    state: &mut self.state.file_picker,
                                    anchor_rect: prompt_layout.input_rect,
                                });
                            }
                        });
                }

                egui::Frame::new()
                    .fill(colors.bg_body)
                    .corner_radius(styles.radii.md)
                    .stroke(egui::Stroke::new(styles.sizes.border, colors.border))
                    .inner_margin(egui::Margin::same(styles.spacing.medium as i8))
                    .show(ui, |ui| {
                        ctx.bind::<egui::Ui>(ui).add(editor::PromptEditorWidget {
                            state: &mut *self.state,
                            sid: self.sid,
                            layout: prompt_layout,
                            commands: self.commands,
                            filtered_commands: &filtered_commands,
                            is_chat_mode,
                            address_bar_editing: self.address_bar_editing,
                            show_suggestions: show_command_suggestions,
                        });
                    });
            });
    }
}

impl PromptWidget<'_> {
    /// Filter the command list for the `:` prefix query.
    /// Returns `(show_suggestions, filtered_commands, suggestion_panel_height)`.
    ///
    /// Uses fuzzy matching: query characters must appear in order within the
    /// label or action string, but not necessarily contiguously. Results are
    /// ranked by match quality (consecutive runs, position of first match).
    fn compute_command_suggestions<'a>(
        text: &str,
        commands: &'a [CommandEntry],
    ) -> (bool, Vec<(usize, &'a CommandEntry)>) {
        let colon_prefix = text.starts_with(':');
        let suggestion_query = if colon_prefix {
            text[1..].to_lowercase()
        } else {
            String::new()
        };
        let mut scored: Vec<(usize, &CommandEntry, i32)> = if colon_prefix {
            commands
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    if suggestion_query.is_empty() {
                        return Some((i, c, 0));
                    }
                    let label_score = fuzzy_score(&suggestion_query, &c.label.to_lowercase());
                    let action_score = fuzzy_score(&suggestion_query, &c.action.to_lowercase());
                    let best = label_score.max(action_score);
                    if best > 0 { Some((i, c, best)) } else { None }
                })
                .collect()
        } else {
            Vec::new()
        };
        // Sort by descending score so best matches appear first.
        scored.sort_by_key(|b| std::cmp::Reverse(b.2));
        let filtered_cmds: Vec<(usize, &CommandEntry)> =
            scored.into_iter().map(|(i, c, _)| (i, c)).collect();
        let show = colon_prefix && !filtered_cmds.is_empty();
        (show, filtered_cmds)
    }
}
