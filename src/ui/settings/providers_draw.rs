use super::providers::*;
use super::*;

impl ProvidersSettingsState {
    pub fn draw(&mut self, ui: &mut egui::Ui, styles: &Styles, colors: &Colors) -> Vec<UiAction> {
        let mut actions: Vec<UiAction> = Vec::new();

        let heading_color = colors.text_title;
        let body_color = colors.text_caption;
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let label_size = styles.typography.caption0;
        let accent = colors.primary;

        ui.label(
            egui::RichText::new("LLM Providers")
                .color(heading_color)
                .size(heading_size)
                .strong(),
        );
        ui.add_space(styles.spacing.medium);
        ui.label(
            egui::RichText::new(
                "Configure LLM provider endpoints. These are used to fetch available models \
             and as connection targets when a profile selects a provider.",
            )
            .color(body_color)
            .size(label_size),
        );
        ui.add_space(styles.spacing.medium);

        // ── Keychain status indicator ─────────────────────────────────────────
        if self.keychain_available {
            ui.label(
                egui::RichText::new("🔐 System keychain available")
                    .color(body_color)
                    .size(label_size),
            );
        } else {
            ui.label(
                egui::RichText::new("⚠ System keychain not available — using env vars only")
                    .color(colors.warning)
                    .size(label_size),
            );
        }
        ui.add_space(styles.spacing.small);

        // ── Provider list ─────────────────────────────────────────────────────
        let mut delete_idx: Option<usize> = None;
        let mut edit_idx: Option<usize> = None;

        for (idx, entry) in self.providers.iter().enumerate() {
            let is_editing = self.editing_index == Some(idx);
            let bg = if is_editing {
                ui.visuals().selection.bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };

            egui::Frame::new()
                .fill(bg)
                .corner_radius(egui::CornerRadius::same(styles.radii.md as _))
                .inner_margin(egui::Margin::symmetric(
                    styles.spacing.medium as _,
                    styles.spacing.small as _,
                ))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Type badge
                        ui.colored_label(accent, format!("[{}]", entry.provider_type));
                        ui.label(
                            egui::RichText::new(&entry.name)
                                .color(heading_color)
                                .size(body_size),
                        );
                        if !entry.api_base.is_empty() {
                            ui.label(
                                egui::RichText::new(&entry.api_base)
                                    .color(body_color)
                                    .size(label_size),
                            );
                        }

                        // Secret storage status badge
                        if entry.has_keychain_secret {
                            ui.label(egui::RichText::new("🔐").size(label_size))
                                .on_hover_text("API key stored in system keychain");
                        } else if !entry.api_key_env.is_empty() {
                            ui.label(egui::RichText::new("🔑").size(label_size))
                                .on_hover_text(format!(
                                    "API key from env var ${}",
                                    entry.api_key_env
                                ));
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("✕")
                                            .color(colors.warning)
                                            .size(label_size),
                                    )
                                    .fill(egui::Color32::TRANSPARENT),
                                )
                                .clicked()
                            {
                                delete_idx = Some(idx);
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Edit").color(accent).size(label_size),
                                    )
                                    .fill(egui::Color32::TRANSPARENT),
                                )
                                .clicked()
                            {
                                edit_idx = Some(idx);
                            }
                        });
                    });
                });
        }

        // Process delete.
        if let Some(idx) = delete_idx {
            self.providers.remove(idx);
            if self.editing_index == Some(idx) {
                self.editing_index = None;
            }
            actions.push(UiAction::SaveProviders);
        }

        // Process edit click — load fields.
        if let Some(idx) = edit_idx {
            let entry = &self.providers[idx];
            self.edit_name = entry.name.clone();
            self.edit_provider_type = entry.provider_type.clone();
            self.edit_api_base = entry.api_base.clone();
            self.edit_api_key_env = entry.api_key_env.clone();
            self.editing_index = Some(idx);
            self.adding_new = false;
        }

        ui.add_space(styles.spacing.medium);

        // ── Add new button ────────────────────────────────────────────────────
        if !self.adding_new
            && self.editing_index.is_none()
            && ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("+ Add Provider")
                            .color(accent)
                            .size(body_size),
                    )
                    .fill(egui::Color32::TRANSPARENT),
                )
                .clicked()
        {
            self.adding_new = true;
            self.editing_index = None;
            self.edit_provider_type = "openai".into();
            let (name, base, env) = provider_defaults("openai");
            self.edit_name = name.into();
            self.edit_api_base = base.into();
            self.edit_api_key_env = env.into();
        }

        // ── Edit / Add form ───────────────────────────────────────────────────
        if self.adding_new || self.editing_index.is_some() {
            ui.add_space(styles.spacing.medium);
            ui.separator();
            ui.add_space(styles.spacing.medium);

            let form_title = if self.adding_new {
                "New Provider"
            } else {
                "Edit Provider"
            };
            ui.label(
                egui::RichText::new(form_title)
                    .color(heading_color)
                    .size(body_size)
                    .strong(),
            );
            ui.add_space(styles.spacing.medium);

            // Name
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Name:")
                        .color(body_color)
                        .size(label_size),
                );
                ui.text_edit_singleline(&mut self.edit_name);
            });

            // Provider type dropdown
            let prev_type = self.edit_provider_type.clone();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Type:")
                        .color(body_color)
                        .size(label_size),
                );
                egui::ComboBox::from_id_salt("provider_type_combo")
                    .selected_text(&self.edit_provider_type)
                    .show_ui(ui, |ui| {
                        for &pt in PROVIDER_TYPES {
                            ui.selectable_value(&mut self.edit_provider_type, pt.to_string(), pt);
                        }
                    });
            });
            if self.edit_provider_type != prev_type {
                let (name, base, env) = provider_defaults(&self.edit_provider_type);
                self.edit_name = name.into();
                self.edit_api_base = base.into();
                self.edit_api_key_env = env.into();
            }

            // API Base
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("API Base:")
                        .color(body_color)
                        .size(label_size),
                );
                ui.text_edit_singleline(&mut self.edit_api_base);
            });

            // API Key Env
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("API Key Env:")
                        .color(body_color)
                        .size(label_size),
                );
                ui.text_edit_singleline(&mut self.edit_api_key_env);
            });

            // ── Keychain Controls ─────────────────────────────────────────────
            if self.keychain_available && !self.edit_name.is_empty() {
                ui.add_space(styles.spacing.medium);
                let key_name = crate::services::secret::provider_api_key(&self.edit_name);
                let store = &self.secret_store;
                let has_stored = store
                    .resolve(&key_name, None, &crate::services::secret::SecretStorage::Keychain)
                    .ok()
                    .flatten()
                    .is_some();

                ui.horizontal(|ui| {
                    if has_stored {
                        ui.label(
                            egui::RichText::new("🔐 Stored in keychain")
                                .color(colors.success)
                                .size(label_size),
                        );
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Delete from Keychain")
                                        .color(colors.danger)
                                        .size(label_size),
                                )
                                .fill(egui::Color32::TRANSPARENT),
                            )
                            .clicked()
                            && let Err(e) = store.delete(&key_name)
                        {
                            log::error!("Failed to delete keychain secret: {}", e);
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("API Key:")
                                .color(body_color)
                                .size(label_size),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.keychain_secret_input)
                                .hint_text("Enter API key...")
                                .password(true),
                        );
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Store in Keychain")
                                        .color(accent)
                                        .size(label_size),
                                )
                                .fill(egui::Color32::TRANSPARENT),
                            )
                            .clicked()
                            && !self.keychain_secret_input.is_empty()
                        {
                            if let Err(e) = store.store(&key_name, &self.keychain_secret_input) {
                                log::error!("Failed to store keychain secret: {}", e);
                            }
                            self.keychain_secret_input.clear();
                        }
                    }
                });
            }

            ui.add_space(styles.spacing.medium);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Save").color(accent).size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    let entry = ProviderEntry {
                        name: self.edit_name.clone(),
                        provider_type: self.edit_provider_type.clone(),
                        api_base: self.edit_api_base.clone(),
                        api_key_env: self.edit_api_key_env.clone(),
                        has_keychain_secret: false,
                    };

                    if let Some(idx) = self.editing_index {
                        // Update existing.
                        if idx < self.providers.len() {
                            self.providers[idx] = entry;
                        }
                    } else {
                        // Add new.
                        self.providers.push(entry);
                    }
                    self.adding_new = false;
                    self.editing_index = None;
                    actions.push(UiAction::SaveProviders);
                }

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Cancel")
                                .color(body_color)
                                .size(body_size),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.adding_new = false;
                    self.editing_index = None;
                }
            });
        }

        // ── Save status indicator ─────────────────────────────────────────────
        if let Some((ref msg, timestamp)) = self.save_status {
            let elapsed = ui.ctx().input(|i| i.time) - timestamp;
            if elapsed < 3.0 {
                ui.add_space(styles.spacing.medium);
                ui.colored_label(colors.primary, msg);
            } else {
                self.save_status = None;
            }
        }

        actions
    }
} // impl ProvidersSettingsState
