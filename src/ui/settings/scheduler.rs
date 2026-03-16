//! Settings > Scheduled Tasks section — list, create, edit, history.

use crate::ai::scheduler::{ExecutionRecord, ScheduledTaskStore, cron_description, validate_cron};
use crate::ui::actions::UiAction;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// Persistent UI state for the Scheduled Tasks settings section.
#[derive(Debug, Clone, Default)]
pub struct SchedulerSettingsState {
    /// ID of the task selected for editing (None = list view).
    pub selected_task_id: Option<String>,
    /// Whether we're creating a new task.
    pub creating_new: bool,
    /// Whether we're viewing history for a task.
    pub viewing_history_for: Option<String>,
    /// Confirmation dialog for deletion.
    pub confirm_delete: Option<String>,

    // Editor fields.
    pub edit_name: String,
    pub edit_cron: String,
    pub edit_action_type: String,
    pub edit_action_value: String,
    pub edit_agent_name: String,
    pub edit_enabled: bool,

    /// ID of task whose fields are currently loaded.
    pub fields_loaded_for: Option<String>,
}

impl SchedulerSettingsState {
    /// Reset editor fields for a new task.
    pub fn reset_editor(&mut self) {
        self.edit_name.clear();
        self.edit_cron = "0 9 * * *".to_string();
        self.edit_action_type = "prompt".to_string();
        self.edit_action_value.clear();
        self.edit_agent_name.clear();
        self.edit_enabled = true;
        self.fields_loaded_for = None;
    }

    /// Draw the Scheduled Tasks section. Returns UI actions.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        store: &mut ScheduledTaskStore,
        history: &[ExecutionRecord],
        styles: &Styles,
        colors: &Colors,
    ) -> Vec<UiAction> {
        let mut actions = Vec::new();

        let text_color = colors.text_title;
        let dim_color = colors.text_caption;
        let emphasis_color = colors.primary;
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let caption_size = styles.typography.caption0;

        // ── Viewing history ──────────────────────────────────────────────────
        if let Some(task_id) = self.viewing_history_for.clone() {
            let task_name = store
                .get(&task_id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| task_id.clone());

            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("← Back")
                                .color(emphasis_color)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.viewing_history_for = None;
                }
                ui.label(
                    egui::RichText::new(format!("History: {}", task_name))
                        .color(text_color)
                        .size(heading_size)
                        .strong(),
                );
            });
            ui.add_space(styles.spacing.medium);

            let task_history: Vec<&ExecutionRecord> =
                history.iter().filter(|r| r.task_id == task_id).collect();

            if task_history.is_empty() {
                ui.label(
                    egui::RichText::new("No execution history yet.")
                        .color(dim_color)
                        .size(body_size),
                );
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for record in &task_history {
                        let status_icon = match record.status.as_str() {
                            "success" => "✓",
                            "timeout" => "⏱",
                            _ => "✗",
                        };
                        let status_color = match record.status.as_str() {
                            "success" => colors.success,
                            "timeout" => colors.warning,
                            _ => colors.danger,
                        };

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(status_icon)
                                    .color(status_color)
                                    .size(body_size),
                            );
                            ui.label(
                                egui::RichText::new(&record.timestamp)
                                    .color(dim_color)
                                    .size(caption_size),
                            );
                            ui.label(
                                egui::RichText::new(format!("{}ms", record.duration_ms))
                                    .color(dim_color)
                                    .size(caption_size),
                            );
                        });

                        if !record.output.is_empty() {
                            ui.label(
                                egui::RichText::new(&record.output)
                                    .color(text_color)
                                    .size(caption_size),
                            );
                        }
                        if let Some(err) = &record.error {
                            ui.label(
                                egui::RichText::new(err)
                                    .color(colors.danger)
                                    .size(caption_size),
                            );
                        }
                        ui.separator();
                    }
                });
            }

            return actions;
        }

        // ── Editor view (create or edit) ─────────────────────────────────────
        if self.creating_new || self.selected_task_id.is_some() {
            let is_new = self.creating_new;

            // Load existing task fields if editing.
            if let Some(ref id) = self.selected_task_id
                && self.fields_loaded_for.as_deref() != Some(id.as_str())
                && let Some(task) = store.get(id)
            {
                self.edit_name = task.name.clone();
                self.edit_cron = task.cron.clone();
                self.edit_action_type = task.action_type.clone();
                self.edit_action_value = task.action_value.clone();
                self.edit_agent_name = task.agent_name.clone();
                self.edit_enabled = task.enabled;
                self.fields_loaded_for = Some(id.clone());
            }

            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("← Back")
                                .color(emphasis_color)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.creating_new = false;
                    self.selected_task_id = None;
                    self.fields_loaded_for = None;
                }
                let title = if is_new { "New Task" } else { "Edit Task" };
                ui.label(
                    egui::RichText::new(title)
                        .color(text_color)
                        .size(heading_size)
                        .strong(),
                );
            });
            ui.add_space(styles.spacing.medium);

            // Name.
            ui.label(
                egui::RichText::new("Name")
                    .color(dim_color)
                    .size(caption_size),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.edit_name)
                    .desired_width(styles.typography.line_height * 14.0)
                    .text_color(text_color),
            );
            ui.add_space(styles.spacing.medium);

            // Cron expression.
            ui.label(
                egui::RichText::new("Cron Expression")
                    .color(dim_color)
                    .size(caption_size),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.edit_cron)
                    .desired_width(styles.typography.line_height * 10.0)
                    .text_color(text_color),
            );
            // Live preview.
            let cron_preview = match validate_cron(&self.edit_cron) {
                Ok(()) => cron_description(&self.edit_cron),
                Err(e) => e,
            };
            ui.label(
                egui::RichText::new(&cron_preview)
                    .color(dim_color)
                    .size(caption_size)
                    .italics(),
            );
            ui.add_space(styles.spacing.medium);

            // Action type.
            ui.label(
                egui::RichText::new("Action Type")
                    .color(dim_color)
                    .size(caption_size),
            );
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.edit_action_type, "prompt".to_string(), "Prompt");
                ui.selectable_value(&mut self.edit_action_type, "command".to_string(), "Command");
            });
            ui.add_space(styles.spacing.small);

            // Action value.
            let value_label = if self.edit_action_type == "command" {
                "Shell Command"
            } else {
                "Prompt Text"
            };
            ui.label(
                egui::RichText::new(value_label)
                    .color(dim_color)
                    .size(caption_size),
            );
            ui.add(
                egui::TextEdit::multiline(&mut self.edit_action_value)
                    .desired_width(styles.typography.line_height * 18.0)
                    .desired_rows(3)
                    .text_color(text_color),
            );
            ui.add_space(styles.spacing.medium);

            // Agent name (optional).
            ui.label(
                egui::RichText::new("Agent (optional)")
                    .color(dim_color)
                    .size(caption_size),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.edit_agent_name)
                    .desired_width(styles.typography.line_height * 10.0)
                    .text_color(text_color),
            );
            ui.add_space(styles.spacing.medium);

            // Enabled toggle.
            ui.checkbox(&mut self.edit_enabled, "Enabled");
            ui.add_space(styles.spacing.medium);

            // Action buttons.
            ui.horizontal(|ui| {
                let can_save = !self.edit_name.is_empty() && validate_cron(&self.edit_cron).is_ok();

                if ui
                    .add_enabled(
                        can_save,
                        egui::Button::new(
                            egui::RichText::new(if is_new { "Create" } else { "Save" })
                                .color(text_color)
                                .size(body_size),
                        ),
                    )
                    .clicked()
                {
                    if is_new {
                        actions.push(UiAction::CreateScheduledTask);
                    } else if let Some(id) = self.selected_task_id.clone() {
                        actions.push(UiAction::SaveScheduledTask(id));
                    }
                }
            });

            return actions;
        }

        // ── List view ────────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("🕐 Scheduled Tasks")
                    .color(text_color)
                    .size(heading_size)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("+ New Task")
                                .color(emphasis_color)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.creating_new = true;
                    self.reset_editor();
                }
            });
        });
        ui.add_space(styles.spacing.medium);

        let tasks = store.list().to_vec();

        if tasks.is_empty() {
            ui.label(
                egui::RichText::new("No scheduled tasks yet. Click \"+ New Task\" to create one.")
                    .color(dim_color)
                    .size(body_size),
            );
            return actions;
        }

        // Delete confirmation.
        if let Some(ref del_id) = self.confirm_delete.clone() {
            let del_name = tasks
                .iter()
                .find(|t| t.id == *del_id)
                .map(|t| t.name.as_str())
                .unwrap_or(del_id.as_str());

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Delete \"{}\"?", del_name))
                        .color(colors.danger)
                        .size(body_size),
                );
                if ui
                    .add(egui::Button::new("Yes, Delete").fill(egui::Color32::TRANSPARENT))
                    .clicked()
                {
                    actions.push(UiAction::DeleteScheduledTask(del_id.clone()));
                    self.confirm_delete = None;
                }
                if ui
                    .add(egui::Button::new("Cancel").fill(egui::Color32::TRANSPARENT))
                    .clicked()
                {
                    self.confirm_delete = None;
                }
            });
            ui.add_space(styles.spacing.small);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for task in &tasks {
                let status_icon = if task.enabled { "●" } else { "○" };
                let status_color = if task.enabled {
                    colors.success
                } else {
                    dim_color
                };

                ui.horizontal(|ui| {
                    // Status indicator.
                    ui.label(
                        egui::RichText::new(status_icon)
                            .color(status_color)
                            .size(body_size),
                    );

                    // Name (clickable to edit).
                    let name_resp = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&task.name)
                                .color(text_color)
                                .size(body_size),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if name_resp.clicked() {
                        self.selected_task_id = Some(task.id.clone());
                        self.fields_loaded_for = None;
                    }

                    // Cron description.
                    ui.label(
                        egui::RichText::new(cron_description(&task.cron))
                            .color(dim_color)
                            .size(caption_size),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Delete button.
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("🗑").color(dim_color).size(caption_size),
                                )
                                .fill(egui::Color32::TRANSPARENT),
                            )
                            .clicked()
                        {
                            self.confirm_delete = Some(task.id.clone());
                        }

                        // History button.
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("📋")
                                        .color(dim_color)
                                        .size(caption_size),
                                )
                                .fill(egui::Color32::TRANSPARENT),
                            )
                            .clicked()
                        {
                            self.viewing_history_for = Some(task.id.clone());
                        }

                        // Enable/disable toggle.
                        let toggle_label = if task.enabled { "Disable" } else { "Enable" };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(toggle_label)
                                        .color(dim_color)
                                        .size(caption_size),
                                )
                                .fill(egui::Color32::TRANSPARENT),
                            )
                            .clicked()
                        {
                            actions.push(UiAction::ToggleScheduledTask(task.id.clone()));
                        }
                    });
                });
                ui.separator();
            }
        });

        actions
    }
}
