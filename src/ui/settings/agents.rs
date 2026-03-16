//! Settings > Agents & Skills section — agent list with enable/disable/install/uninstall.
#![allow(dead_code)]

use crate::ai::agent::AgentRegistry;
use crate::ui::actions::UiAction;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// Transient state for the Agents & Skills settings section.
#[derive(Debug, Clone, Default)]
pub struct AgentsSettingsState {
    /// Agent pending uninstall confirmation (name).
    pub confirm_uninstall: Option<String>,
    /// Whether an install operation is pending.
    pub install_pending: bool,
}

impl AgentsSettingsState {
    /// Draw the Agents & Skills section content inside the settings right panel.
    ///
    /// Returns a list of actions to perform (install, uninstall, toggle).
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        _registry: &mut AgentRegistry,
        styles: &Styles,
        colors: &Colors,
        skills_panel_state: Option<&mut crate::ui::skills_panel::SkillsPanelState>,
    ) -> Vec<UiAction> {
        let mut actions: Vec<UiAction> = Vec::new();

        let heading_color = colors.text_title;
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let accent = colors.primary;

        // ── Header row: title + Install button ────────────────────────────────
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("🔧 Agents & Skills")
                    .color(heading_color)
                    .size(heading_size)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let install_btn = ui.add(
                    egui::Button::new(
                        egui::RichText::new("Install…")
                            .color(accent)
                            .size(body_size),
                    )
                    .fill(egui::Color32::TRANSPARENT),
                );
                if install_btn.clicked() {
                    self.install_pending = true;
                }
            });
        });

        ui.add_space(styles.spacing.medium + styles.spacing.small);

        // // ── Agent list ────────────────────────────────────────────────────────
        // let agents = registry.list().to_vec();

        // if agents.is_empty() {
        //     ui.add_space(styles.spacing.large + styles.spacing.medium);
        //     ui.label(
        //         egui::RichText::new("No agents installed.\nClick [Install…] to add an agent.")
        //             .color(body_color)
        //             .size(body_size),
        //     );
        // } else {
        //     egui::ScrollArea::vertical()
        //         .auto_shrink([false, false])
        //         .show(ui, |ui| {
        //             for entry in &agents {
        //                 let manifest = registry.lookup(&entry.name).cloned();
        //                 self.draw_agent_row(
        //                     ui,
        //                     entry,
        //                     manifest.as_ref(),
        //                     styles,
        //                     colors,
        //                     &mut actions,
        //                 );
        //                 ui.add_space(styles.spacing.small);
        //                 ui.separator();
        //                 ui.add_space(styles.spacing.small);
        //             }
        //         });
        // }

        // // ── Handle install pending ────────────────────────────────────────────
        // if self.install_pending {
        //     self.install_pending = false;
        //     actions.push(UiAction::InstallAgent);
        // }

        // // ── Handle uninstall confirmation ─────────────────────────────────────
        // if let Some(ref name) = self.confirm_uninstall.clone() {
        //     egui::Window::new("Confirm Uninstall")
        //         .collapsible(false)
        //         .resizable(false)
        //         .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        //         .show(ui.ctx(), |ui| {
        //             ui.label(
        //                 egui::RichText::new(format!("Uninstall agent '{}'?", name))
        //                     .color(heading_color)
        //                     .size(body_size),
        //             );
        //             ui.add_space(styles.spacing.medium);
        //             ui.horizontal(|ui| {
        //                 if ui
        //                     .add(egui::Button::new("Cancel").fill(egui::Color32::TRANSPARENT))
        //                     .clicked()
        //                 {
        //                     self.confirm_uninstall = None;
        //                 }
        //                 if ui
        //                     .add(
        //                         egui::Button::new(
        //                             egui::RichText::new("Uninstall").color(colors.danger),
        //                         )
        //                         .fill(egui::Color32::TRANSPARENT),
        //                     )
        //                     .clicked()
        //                 {
        //                     actions.push(UiAction::UninstallAgent(name.clone()));
        //                     self.confirm_uninstall = None;
        //                 }
        //             });
        //         });
        // }

        // ── Skills sub-section ────────────────────────────────────────────────
        if let Some(panel) = skills_panel_state {
            ui.add_space(styles.spacing.large);
            ui.separator();
            ui.add_space(styles.spacing.medium + styles.spacing.small);

            ui.label(
                egui::RichText::new("📦 Skills")
                    .color(heading_color)
                    .size(heading_size)
                    .strong(),
            );
            ui.add_space(styles.spacing.medium);

            let skill_actions = panel.show(ui, styles, colors);
            actions.extend(skill_actions);
        }

        actions
    }

    /// Draw a single agent row with name, version, description, skills, and buttons.
    fn draw_agent_row(
        &mut self,
        ui: &mut egui::Ui,
        entry: &crate::ai::agent::InstalledAgent,
        manifest: Option<&crate::ai::agent::AgentManifest>,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        let heading_color = colors.text_title;
        let body_color = colors.text_caption;
        let body_size = styles.typography.body2;
        let accent = colors.primary;

        // First line: status icon + name + version + buttons
        ui.horizontal(|ui| {
            // Enabled/disabled status icon.
            let status = if entry.enabled { "✅" } else { "⬜" };
            ui.label(egui::RichText::new(status).size(body_size));

            // Agent name.
            ui.label(
                egui::RichText::new(&entry.name)
                    .color(heading_color)
                    .size(body_size)
                    .strong(),
            );

            // Version from manifest.
            if let Some(m) = manifest {
                ui.label(
                    egui::RichText::new(format!("v{}", m.agent.version))
                        .color(body_color)
                        .size(body_size),
                );
            }

            // Push buttons to the right.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Uninstall button.
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Uninstall")
                                .color(colors.danger)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.confirm_uninstall = Some(entry.name.clone());
                }

                // Enable/Disable toggle.
                let toggle_label = if entry.enabled { "Disable" } else { "Enable" };
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(toggle_label)
                                .color(accent)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    actions.push(UiAction::ToggleAgent(entry.name.clone()));
                }
            });
        });

        // Description line.
        if let Some(m) = manifest {
            if !m.agent.description.is_empty() {
                ui.label(
                    egui::RichText::new(format!("   {}", m.agent.description))
                        .color(body_color)
                        .size(body_size),
                );
            }

            // Skills line.
            if !m.skills.is_empty() {
                let skill_names: Vec<&str> = m.skills.iter().map(|s| s.name.as_str()).collect();
                ui.label(
                    egui::RichText::new(format!("   Skills: {}", skill_names.join(", ")))
                        .color(body_color)
                        .size(body_size),
                );
            }

            // Schedule line (if present).
            if let Some(ref sched) = m.schedule {
                ui.label(
                    egui::RichText::new(format!("   Schedule: {}", sched.cron))
                        .color(body_color)
                        .size(body_size),
                );
            }
        }
    }
}
