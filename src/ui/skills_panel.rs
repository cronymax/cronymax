//! Skills management panel — egui UI for browsing, installing, and managing
//! OpenClaw-compatible skills from the local filesystem and ClawHub registry.
#![allow(dead_code)]

use egui::{self, RichText, Ui};

use crate::ai::clawhub::ClawHubSkillResult;
use crate::ai::skills::manager::InstalledSkillEntry;
use crate::ui::UiAction;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

// ── Panel State ──────────────────────────────────────────────────────────────

/// Which tab is active in the skills panel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SkillsPanelTab {
    #[default]
    Installed,
    BrowseClawHub,
}

/// State for the skills management panel.
#[derive(Debug, Default)]
pub struct SkillsPanelState {
    /// Currently active tab.
    pub active_tab: SkillsPanelTab,
    /// Search query for ClawHub.
    pub search_query: String,
    /// Latest ClawHub search results.
    pub search_results: Vec<ClawHubSkillResult>,
    /// Installed skills list (name, entry).
    pub installed_list: Vec<(String, InstalledSkillEntry)>,
    /// Currently selected skill (for detail view).
    pub selected_skill: Option<String>,
    /// Whether a search is in progress.
    pub search_in_progress: bool,
    /// Error message from last operation.
    pub last_error: Option<String>,
    /// Whether the panel is open.
    pub open: bool,
    /// Whether an update check has found updates (name → new version).
    pub update_available: Vec<(String, String)>,
}

impl SkillsPanelState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Render the skills panel. Returns actions the app should execute.
    pub fn show(&mut self, ui: &mut Ui, styles: &Styles, colors: &Colors) -> Vec<UiAction> {
        let mut actions = Vec::new();

        // Error banner.
        let mut clear_error = false;
        if let Some(err) = self.last_error.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(colors.danger, format!("⚠ {}", err));
                if ui.small_button("✕").clicked() {
                    clear_error = true;
                }
            });
            ui.add_space(styles.spacing.small);
        }
        if clear_error {
            self.last_error = None;
        }

        // Tab bar.
        ui.horizontal(|ui| {
            if ui
                .selectable_label(
                    self.active_tab == SkillsPanelTab::Installed,
                    RichText::new("📦 Installed").strong(),
                )
                .clicked()
            {
                self.active_tab = SkillsPanelTab::Installed;
            }
            if ui
                .selectable_label(
                    self.active_tab == SkillsPanelTab::BrowseClawHub,
                    RichText::new("🔍 Browse ClawHub").strong(),
                )
                .clicked()
            {
                self.active_tab = SkillsPanelTab::BrowseClawHub;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("↻ Reload").clicked() {
                    actions.push(UiAction::ReloadSkills);
                }
            });
        });

        ui.separator();

        match self.active_tab {
            SkillsPanelTab::Installed => {
                actions.extend(self.render_installed_tab(ui, styles, colors));
            }
            SkillsPanelTab::BrowseClawHub => {
                actions.extend(self.render_browse_tab(ui, styles, colors));
            }
        }

        actions
    }

    // ── Installed Tab ────────────────────────────────────────────────────

    fn render_installed_tab(
        &mut self,
        ui: &mut Ui,
        styles: &Styles,
        colors: &Colors,
    ) -> Vec<UiAction> {
        let mut actions = Vec::new();

        // Update all button if updates available.
        if !self.update_available.is_empty() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} update(s) available",
                        self.update_available.len()
                    ))
                    .color(colors.info),
                );
            });
            ui.add_space(styles.spacing.small);
        }

        if self.installed_list.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(styles.spacing.large * 2.5);
                ui.label(
                    RichText::new("No skills installed")
                        .color(colors.text_caption)
                        .size(styles.typography.title5),
                );
                ui.add_space(styles.spacing.medium);
                ui.label("Browse ClawHub to find and install skills.");
                ui.add_space(styles.spacing.medium);
                if ui.button("Browse ClawHub →").clicked() {
                    self.active_tab = SkillsPanelTab::BrowseClawHub;
                }
            });
            return actions;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let list = self.installed_list.clone();
                for (name, entry) in &list {
                    self.render_installed_skill_row(ui, name, entry, styles, colors, &mut actions);
                    ui.separator();
                }
            });

        actions
    }

    fn render_installed_skill_row(
        &mut self,
        ui: &mut Ui,
        name: &str,
        entry: &InstalledSkillEntry,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        ui.horizontal(|ui| {
            // Name + version.
            ui.label(
                RichText::new(name)
                    .strong()
                    .size(styles.typography.headline),
            );
            ui.label(
                RichText::new(format!("v{}", entry.version))
                    .color(colors.text_caption)
                    .size(styles.typography.caption1),
            );

            // Source badge.
            let badge = match entry.source.as_str() {
                "clawhub" => RichText::new(" ClawHub ")
                    .background_color(colors.primary)
                    .color(colors.text_title)
                    .size(styles.typography.caption1),
                _ => RichText::new(" Local ")
                    .color(colors.text_title)
                    .size(styles.typography.caption1),
            };
            ui.label(badge);

            // Update available indicator.
            if self.update_available.iter().any(|(n, _)| n == name) {
                ui.label(
                    RichText::new("⬆ update")
                        .color(colors.info)
                        .size(styles.typography.caption1),
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Uninstall button.
                if ui
                    .button(RichText::new("🗑").size(styles.typography.headline))
                    .on_hover_text("Uninstall skill")
                    .clicked()
                {
                    actions.push(UiAction::UninstallSkill(name.to_string()));
                }

                // Enable/disable toggle.
                let mut enabled = entry.enabled;
                if ui.checkbox(&mut enabled, "").changed() {
                    actions.push(UiAction::ToggleSkill {
                        name: name.to_string(),
                        enabled,
                    });
                }

                ui.label(if entry.enabled {
                    RichText::new("Enabled")
                        .color(colors.success)
                        .size(styles.typography.caption1)
                } else {
                    RichText::new("Disabled")
                        .color(colors.text_caption)
                        .size(styles.typography.caption1)
                });
            });
        });
    }

    // ── Browse ClawHub Tab ───────────────────────────────────────────────

    fn render_browse_tab(
        &mut self,
        ui: &mut Ui,
        styles: &Styles,
        colors: &Colors,
    ) -> Vec<UiAction> {
        let mut actions = Vec::new();

        // Search bar.
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search ClawHub skills…")
                    .desired_width(ui.available_width() - styles.spacing.large * 5.0),
            );

            let search_clicked = ui.button("Search").clicked();
            let enter_pressed = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            if (search_clicked || enter_pressed)
                && !self.search_query.is_empty()
                && !self.search_in_progress
            {
                actions.push(UiAction::SearchSkills(self.search_query.clone()));
                self.search_in_progress = true;
            }
        });

        ui.add_space(styles.spacing.small);

        if self.search_in_progress {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Searching ClawHub…");
            });
            return actions;
        }

        if self.search_results.is_empty() && !self.search_query.is_empty() {
            ui.label(RichText::new("No results").color(colors.text_caption));
            return actions;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let results = self.search_results.clone();
                for result in &results {
                    self.render_search_result_row(ui, result, styles, colors, &mut actions);
                    ui.separator();
                }
            });

        actions
    }

    fn render_search_result_row(
        &self,
        ui: &mut Ui,
        result: &ClawHubSkillResult,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&result.name)
                            .strong()
                            .size(styles.typography.headline),
                    );
                    ui.label(
                        RichText::new(format!("v{}", result.version))
                            .color(colors.text_caption)
                            .size(styles.typography.caption1),
                    );
                });

                ui.label(&result.description);

                ui.horizontal(|ui| {
                    if let Some(author) = &result.author {
                        ui.label(
                            RichText::new(format!("by {}", author))
                                .color(colors.text_caption)
                                .size(styles.typography.caption1),
                        );
                    }
                    ui.label(
                        RichText::new(format!("⭐ {}", result.stars))
                            .size(styles.typography.caption1)
                            .color(colors.warning),
                    );
                    ui.label(
                        RichText::new(format!("↓ {}", result.install_count))
                            .size(styles.typography.caption1)
                            .color(colors.text_caption),
                    );
                    // Tags.
                    for tag in result.tags.iter().take(3) {
                        ui.label(
                            RichText::new(tag)
                                .color(colors.text_title)
                                .size(styles.typography.caption1),
                        );
                    }
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let already_installed = self
                    .installed_list
                    .iter()
                    .any(|(n, _)| *n == result.slug || *n == result.name);

                if already_installed {
                    ui.label(
                        RichText::new("Installed ✓")
                            .color(colors.success)
                            .size(styles.typography.body2),
                    );
                } else if ui.button(RichText::new("Install").strong()).clicked() {
                    actions.push(UiAction::InstallSkill(result.slug.clone()));
                }
            });
        });
    }
}
