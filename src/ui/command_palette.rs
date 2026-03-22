//! Command palette — fuzzy-search overlay for actions and commands.
//!
//! Triggered by `Cmd+Shift+P` / `Ctrl+Shift+P` (mapped to `KeyAction::CommandMode`).
//! Shows a centered overlay popup with all available commands, fuzzy-filtered
//! by the query text. Uses `nucleo` for fast matching.

use crate::ui::actions::UiAction;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// A single entry in the command palette.
#[derive(Debug, Clone)]
pub struct PaletteEntry {
    /// Display label (e.g. "New Terminal").
    pub label: String,
    /// Action to execute when selected.
    pub action: PaletteAction,
    /// Optional keyboard shortcut hint.
    pub shortcut: Option<String>,
}

/// The action a palette entry triggers.
#[derive(Debug, Clone)]
pub enum PaletteAction {
    /// Dispatch a UiAction.
    Ui(UiAction),
    /// Execute a colon command string.
    ColonCommand(String),
}

/// State for the command palette overlay.
#[derive(Debug)]
pub struct CommandPaletteState {
    /// Whether the palette is visible.
    pub open: bool,
    /// Current fuzzy query text.
    pub query: String,
    /// Index of the selected entry in filtered results.
    pub selected: usize,
    /// All available entries (built once, refreshed on open).
    entries: Vec<PaletteEntry>,
    /// Filtered/scored indices into `entries`.
    filtered: Vec<usize>,
}

impl Default for CommandPaletteState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            entries: build_default_entries(),
            filtered: Vec::new(),
        }
    }
}

impl CommandPaletteState {
    /// Open the palette and reset state.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.refilter();
    }

    /// Close the palette.
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
    }

    /// Update the query and refilter matches.
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.selected = 0;
        self.refilter();
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    /// Get the currently selected entry (if any).
    pub fn selected_entry(&self) -> Option<&PaletteEntry> {
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.entries.get(idx))
    }

    /// Get filtered entries for display.
    pub fn visible_entries(&self) -> Vec<&PaletteEntry> {
        self.filtered
            .iter()
            .filter_map(|&idx| self.entries.get(idx))
            .collect()
    }

    /// Refilter entries based on current query.
    fn refilter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
            return;
        }
        let query_lower = self.query.to_lowercase();
        let mut scored: Vec<(usize, i32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                let label_lower = entry.label.to_lowercase();
                // Simple substring + prefix scoring.
                if label_lower.contains(&query_lower) {
                    let score = if label_lower.starts_with(&query_lower) {
                        100
                    } else {
                        50
                    };
                    Some((idx, score))
                } else {
                    // Fuzzy: check if all query chars appear in order.
                    let mut chars = query_lower.chars().peekable();
                    for c in label_lower.chars() {
                        if chars.peek() == Some(&c) {
                            chars.next();
                        }
                    }
                    if chars.peek().is_none() {
                        Some((idx, 10))
                    } else {
                        None
                    }
                }
            })
            .collect();
        scored.sort_by_key(|b| std::cmp::Reverse(b.1));
        self.filtered = scored.into_iter().map(|(idx, _)| idx).collect();
    }

    /// Draw the command palette overlay.
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        styles: &Styles,
        colors: &Colors,
    ) -> Option<PaletteAction> {
        if !self.open {
            return None;
        }

        let mut result = None;
        let screen = ctx.screen_rect();
        let palette_width = 500.0_f32.min(screen.width() - 40.0);
        let palette_x = (screen.width() - palette_width) / 2.0;
        let palette_y = screen.height() * 0.15;

        // Background dimmer.
        let dimmer = egui::Area::new(egui::Id::new("palette_dimmer"))
            .fixed_pos(screen.min)
            .order(egui::Order::Foreground)
            .interactable(true);

        dimmer.show(ctx, |ui| {
            let resp = ui.allocate_response(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(screen, 0.0, colors.bg_mask);
            if resp.clicked() {
                self.close();
            }
        });

        let area = egui::Area::new(egui::Id::new("command_palette"))
            .fixed_pos(egui::Pos2::new(palette_x, palette_y))
            .order(egui::Order::Foreground)
            .interactable(true);

        area.show(ctx, |ui| {
            let frame = egui::Frame::new()
                .fill(colors.bg_float)
                .inner_margin(egui::Margin::from(styles.spacing.medium))
                .corner_radius(styles.radii.md)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(80),
                });

            frame.show(ui, |ui| {
                ui.set_width(palette_width - styles.spacing.medium * 2.0);

                // Search input.
                let input = egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Type a command...")
                    .desired_width(palette_width - styles.spacing.medium * 2.0)
                    .font(egui::TextStyle::Body);

                let resp = ui.add(input);
                if resp.changed() {
                    self.selected = 0;
                    self.refilter();
                }

                // Auto-focus the input.
                resp.request_focus();

                // Handle keyboard navigation.
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.close();
                    return;
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                    self.select_prev();
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                    self.select_next();
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some(entry) = self.selected_entry() {
                        result = Some(entry.action.clone());
                    }
                    self.close();
                    return;
                }

                ui.add_space(styles.spacing.small);

                // Results list – collect owned entries to avoid holding
                // an immutable borrow on `self` inside the closure.
                let visible: Vec<PaletteEntry> =
                    self.visible_entries().into_iter().cloned().collect();
                let max_shown = 12;
                let mut clicked_action: Option<PaletteAction> = None;
                let mut hovered_idx: Option<usize> = None;
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (i, entry) in visible.iter().take(max_shown).enumerate() {
                            let is_selected = i == self.selected;
                            let bg = if is_selected {
                                colors.fill_selected
                            } else {
                                egui::Color32::TRANSPARENT
                            };

                            let frame = egui::Frame::new()
                                .fill(bg)
                                .corner_radius(styles.radii.xs)
                                .inner_margin(egui::Margin::symmetric(
                                    styles.spacing.medium as i8,
                                    styles.spacing.small as i8,
                                ));

                            let resp = frame
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(&entry.label)
                                                .size(styles.typography.body0)
                                                .color(if is_selected {
                                                    colors.text_title
                                                } else {
                                                    colors.text_caption
                                                }),
                                        );
                                        if let Some(shortcut) = &entry.shortcut {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        egui::RichText::new(shortcut)
                                                            .size(styles.typography.caption0)
                                                            .color(colors.text_disabled),
                                                    );
                                                },
                                            );
                                        }
                                    });
                                })
                                .response;

                            if resp.clicked() {
                                clicked_action = Some(entry.action.clone());
                            }
                            if resp.hovered() {
                                hovered_idx = Some(i);
                            }
                        }
                    });

                if let Some(action) = clicked_action {
                    result = Some(action);
                    self.close();
                }
                if let Some(idx) = hovered_idx {
                    self.selected = idx;
                }
            });
        });

        result
    }
}

/// Build the default list of palette entries.
fn build_default_entries() -> Vec<PaletteEntry> {
    vec![
        PaletteEntry {
            label: "New Chat".into(),
            action: PaletteAction::Ui(UiAction::NewChat),
            shortcut: Some("Ctrl+Shift+T".into()),
        },
        PaletteEntry {
            label: "New Terminal".into(),
            action: PaletteAction::Ui(UiAction::NewTerminal),
            shortcut: None,
        },
        PaletteEntry {
            label: "Open Settings".into(),
            action: PaletteAction::Ui(UiAction::OpenSettings),
            shortcut: Some("Cmd+,".into()),
        },
        PaletteEntry {
            label: "Close Settings".into(),
            action: PaletteAction::Ui(UiAction::CloseSettings),
            shortcut: None,
        },
        PaletteEntry {
            label: "Open History".into(),
            action: PaletteAction::Ui(UiAction::OpenHistory),
            shortcut: None,
        },
        PaletteEntry {
            label: "Schedule Tasks".into(),
            action: PaletteAction::Ui(UiAction::OpenScheduler),
            shortcut: None,
        },
        PaletteEntry {
            label: "Split Right".into(),
            action: PaletteAction::Ui(UiAction::SplitRight),
            shortcut: None,
        },
        PaletteEntry {
            label: "Split Down".into(),
            action: PaletteAction::Ui(UiAction::SplitDown),
            shortcut: Some("Ctrl+Shift+D".into()),
        },
        PaletteEntry {
            label: "Open Browser Overlay".into(),
            action: PaletteAction::Ui(UiAction::OpenOverlay),
            shortcut: None,
        },
        PaletteEntry {
            label: "Filter / Search in Terminal".into(),
            action: PaletteAction::ColonCommand("filter".into()),
            shortcut: Some("Cmd+F".into()),
        },
        PaletteEntry {
            label: "Enable Claw Mode".into(),
            action: PaletteAction::Ui(UiAction::EnableClawMode),
            shortcut: None,
        },
        PaletteEntry {
            label: "Disable Claw Mode".into(),
            action: PaletteAction::Ui(UiAction::DisableClawMode),
            shortcut: None,
        },
        PaletteEntry {
            label: "Install Agent".into(),
            action: PaletteAction::Ui(UiAction::InstallAgent),
            shortcut: None,
        },
        PaletteEntry {
            label: "Reload Skills".into(),
            action: PaletteAction::Ui(UiAction::ReloadSkills),
            shortcut: None,
        },
        PaletteEntry {
            label: "Toggle Skills Panel".into(),
            action: PaletteAction::Ui(UiAction::ToggleSkillsPanel),
            shortcut: None,
        },
        PaletteEntry {
            label: "Save LLM Providers".into(),
            action: PaletteAction::Ui(UiAction::SaveProviders),
            shortcut: None,
        },
        PaletteEntry {
            label: "Close Tab".into(),
            action: PaletteAction::ColonCommand("closetab".into()),
            shortcut: Some("Ctrl+Shift+W".into()),
        },
        PaletteEntry {
            label: "Relaunch App".into(),
            action: PaletteAction::Ui(UiAction::RelaunchApp),
            shortcut: None,
        },
    ]
}
