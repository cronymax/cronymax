//! Settings > Profiles section — profile list, detail form, sandbox rules, CRUD operations.

use crate::ui::actions::UiAction;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// Transient state for the Profiles settings section.
#[derive(Debug, Clone)]
pub struct ProfilesSettingsState {
    /// Currently selected profile ID (for detail view).
    pub selected_profile_id: Option<String>,
    /// Whether we're in "new profile" creation mode.
    pub creating_new: bool,
    /// Editable fields — buffered copies of selected profile.
    pub edit_name: String,
    /// Sandbox rules as comma-separated strings for editing.
    pub edit_fs_read: String,
    pub edit_fs_write: String,
    pub edit_fs_deny: String,
    pub edit_network_deny: bool,
    /// Profile pending deletion confirmation.
    pub confirm_delete: Option<String>,
    /// Whether edit fields are loaded for the selected profile.
    pub fields_loaded_for: Option<String>,
    /// Status message shown after save (message, egui time).
    pub save_status: Option<(String, f64)>,
    /// Available models grouped by provider display name: (provider_name, model_id).
    pub available_models: Vec<(String, String)>,
    /// Whether to show a "relaunch required" dialog after sandbox rules changed.
    pub show_relaunch_dialog: bool,
    /// Editable skill category allowlist.
    pub edit_allowed_skills: Vec<String>,
}

impl Default for ProfilesSettingsState {
    fn default() -> Self {
        Self {
            selected_profile_id: None,
            creating_new: false,
            edit_name: String::new(),
            edit_fs_read: "~, /usr, /etc".into(),
            edit_fs_write: "~, /tmp".into(),
            edit_fs_deny: "~/.ssh, ~/.gnupg".into(),
            edit_network_deny: false,
            confirm_delete: None,
            fields_loaded_for: None,
            save_status: None,
            available_models: Vec::new(),
            show_relaunch_dialog: false,
            edit_allowed_skills: crate::profile::Profile::default_allowed_skills(),
        }
    }
}

impl ProfilesSettingsState {
    /// Draw the Profiles section content inside the settings right panel.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        manager: &mut crate::profile::ProfileManager,
        styles: &Styles,
        colors: &Colors,
    ) -> Vec<UiAction> {
        let mut actions: Vec<UiAction> = Vec::new();

        let heading_color = colors.text_title;
        let body_color = colors.text_caption;
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let accent = colors.primary;

        // Auto-select first profile if none selected.
        if self.selected_profile_id.is_none()
            && let Some(p) = manager.active()
        {
            self.selected_profile_id = Some(p.id.clone());
        }

        // Two-panel layout: left list + right detail.
        ui.horizontal(|ui| {
            // ── Left panel: profile list ──────────────────────────────────────
            ui.allocate_ui_with_layout(
                egui::vec2(styles.typography.line_height * 8.0, ui.available_height()),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    Self::draw_profile_list(self, manager, styles, colors, ui);
                },
            );

            ui.separator();

            // ── Right panel: detail form ──────────────────────────────────────
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.available_height()),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    Self::draw_profile_configuration(
                        self,
                        manager,
                        styles,
                        colors,
                        &mut actions,
                        ui,
                    );
                },
            );
        });

        // ── Delete confirmation dialog ────────────────────────────────────────
        // Use egui::Area at Order::Tooltip so it renders above the Foreground settings overlay.
        if let Some(ref pid) = self.confirm_delete.clone() {
            egui::Area::new(egui::Id::new("confirm_delete_dialog"))
                .order(egui::Order::Tooltip)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .corner_radius(egui::CornerRadius::same(styles.radii.md as _))
                        .inner_margin(egui::Margin::same(styles.spacing.large as _))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!("Delete profile '{}'?", pid))
                                    .color(heading_color)
                                    .size(body_size),
                            );
                            ui.add_space(styles.spacing.medium);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new("Cancel")
                                            .fill(egui::Color32::TRANSPARENT),
                                    )
                                    .clicked()
                                {
                                    self.confirm_delete = None;
                                }
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Delete").color(colors.danger),
                                        )
                                        .fill(egui::Color32::TRANSPARENT),
                                    )
                                    .clicked()
                                {
                                    actions.push(UiAction::DeleteProfile(pid.clone()));
                                    self.confirm_delete = None;
                                    self.selected_profile_id = None;
                                    self.fields_loaded_for = None;
                                }
                            });
                        });
                });
        }

        // ── Relaunch-required dialog (sandbox rules changed) ──────────────────
        if self.show_relaunch_dialog {
            egui::Area::new(egui::Id::new("relaunch_sandbox_dialog"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .corner_radius(egui::CornerRadius::same(styles.radii.md as _))
                    .inner_margin(egui::Margin::same(styles.spacing.large as _))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Sandbox rules changed")
                                .color(heading_color)
                                .size(heading_size),
                        );
                        ui.add_space(styles.spacing.small);
                        ui.label(
                            egui::RichText::new(
                                "OS-level sandbox is applied at launch. Relaunch to enforce the new rules.",
                            )
                            .color(body_color)
                            .size(body_size),
                        );
                        ui.add_space(styles.spacing.medium);
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new("Later").fill(egui::Color32::TRANSPARENT))
                                .clicked()
                            {
                                self.show_relaunch_dialog = false;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Relaunch Now").color(accent),
                                    )
                                    .fill(egui::Color32::TRANSPARENT),
                                )
                                .clicked()
                            {
                                self.show_relaunch_dialog = false;
                                actions.push(UiAction::RelaunchApp);
                            }
                        });
                    });
            });
        }

        actions
    }

    fn draw_profile_configuration(
        &mut self,
        manager: &mut crate::profile::ProfileManager,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
        ui: &mut egui::Ui,
    ) {
        let body_color = colors.text_caption;
        let body_size = styles.typography.body2;
        let accent = colors.primary;
        if self.creating_new {
            self.draw_form(ui, None, None, styles, colors, actions);
            ui.add_space(styles.spacing.medium);
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("Create").color(accent).size(body_size))
                        .fill(egui::Color32::TRANSPARENT),
                )
                .clicked()
            {
                actions.push(UiAction::CreateProfile);
            }
        } else if let Some(ref pid) = self.selected_profile_id.clone() {
            // Load fields from the profile if not yet loaded.
            if self.fields_loaded_for.as_deref() != Some(pid)
                && let Some(p) = manager.profiles.get(pid)
            {
                self.load_profile_into_state(p);
                // Load sandbox policy from policy.toml on disk.
                self.load_sandbox_into_state(manager, pid);
                self.fields_loaded_for = Some(pid.clone());
            }

            let is_active = manager.active_profile_id.as_deref() == Some(pid.as_str());
            let profile_path = manager.profile_dir(pid).join("profile.toml");
            self.draw_form(ui, Some(pid), Some(&profile_path), styles, colors, actions);

            ui.add_space(styles.spacing.medium);
            ui.horizontal(|ui| {
                if !is_active
                    && ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Set Active")
                                    .color(accent)
                                    .size(body_size),
                            )
                            .fill(egui::Color32::TRANSPARENT),
                        )
                        .clicked()
                {
                    actions.push(UiAction::SetActiveProfile(pid.clone()));
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Save").color(accent).size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    actions.push(UiAction::SaveProfile(pid.clone()));
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Duplicate")
                                .color(accent)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    actions.push(UiAction::DuplicateProfile(pid.clone()));
                }
                // Delete allowed for all profiles (handler auto-switches active).
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Delete")
                                .color(colors.danger)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.confirm_delete = Some(pid.clone());
                }
            });

            // ── Save status indicator ─────────────────────────────
            if let Some((ref msg, timestamp)) = self.save_status {
                let elapsed = ui.ctx().input(|i| i.time) - timestamp;
                if elapsed < 3.0 {
                    ui.add_space(styles.spacing.medium);
                    ui.colored_label(colors.primary, msg);
                } else {
                    self.save_status = None;
                }
            }
        } else {
            ui.label(
                egui::RichText::new("Select a profile from the list.")
                    .color(body_color)
                    .size(body_size),
            );
        }
    }

    fn draw_profile_list(
        &mut self,
        manager: &mut crate::profile::ProfileManager,
        styles: &Styles,
        colors: &Colors,
        ui: &mut egui::Ui,
    ) {
        let heading_color = colors.text_title;
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let accent = colors.primary;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Profiles")
                    .color(heading_color)
                    .size(heading_size)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("+").color(accent).size(heading_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.creating_new = true;
                    self.selected_profile_id = None;
                    self.fields_loaded_for = None;
                    self.edit_name = "New Profile".into();
                    self.edit_fs_read = "~, /usr, /etc".into();
                    self.edit_fs_write = "~, /tmp".into();
                    self.edit_fs_deny = "~/.ssh, ~/.gnupg".into();
                    self.edit_network_deny = false;
                }
            });
        });

        ui.add_space(styles.spacing.medium);

        let profiles: Vec<(String, String, bool)> = manager
            .list()
            .iter()
            .map(|p| {
                let is_active = manager.active_profile_id.as_deref() == Some(&p.id);
                (p.id.clone(), p.name.clone(), is_active)
            })
            .collect();

        for (pid, pname, is_active) in &profiles {
            let selected = self.selected_profile_id.as_deref() == Some(pid);
            let bg = if selected {
                ui.visuals().selection.bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            let dot = if *is_active { "● " } else { "  " };

            let resp = egui::Frame::new()
                .fill(bg)
                .corner_radius(egui::CornerRadius::same(styles.radii.sm as _))
                .inner_margin(egui::Margin::symmetric(
                    styles.spacing.medium as _,
                    styles.spacing.small as _,
                ))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("{}{}", dot, pname))
                                .color(if selected { accent } else { heading_color })
                                .size(body_size),
                        )
                        .sense(egui::Sense::click()),
                    )
                })
                .inner;

            if resp.clicked() {
                self.selected_profile_id = Some(pid.clone());
                self.creating_new = false;
                self.fields_loaded_for = None; // force reload
            }
        }
    }
}
