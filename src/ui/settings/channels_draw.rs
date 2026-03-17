use super::channels::*;
use super::*;
use crate::channels::ConnectionState;

impl ChannelsSettingsState {
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        profile_names: Option<&[(String, String)]>,
        onboarding: Option<&mut super::onboarding::OnboardingWizardState>,
    ) -> Vec<UiAction> {
        let mut actions = Vec::new();

        let heading_color = colors.text_title;
        let body_color = colors.text_caption;
        let accent = colors.primary;
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let label_size = styles.typography.caption0;

        // ── Heading ──────────────────────────────────────────────────────────
        ui.label(
            egui::RichText::new("Channels")
                .color(heading_color)
                .size(heading_size)
                .strong(),
        );
        ui.add_space(styles.spacing.small);
        ui.label(
            egui::RichText::new("Configure external messaging channels for Claw mode.")
                .color(body_color)
                .size(label_size),
        );
        ui.add_space(styles.spacing.large);

        // ── Feishu / Lark section header with Add button ─────────────────────
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Feishu / Lark")
                    .color(heading_color)
                    .size(body_size)
                    .strong(),
            );
            ui.add_space(styles.spacing.large);
            if ui.button("＋ Add Channel").clicked() {
                actions.push(UiAction::AddLarkChannel);
            }
        });
        ui.label(
            egui::RichText::new("Bidirectional messaging with Feishu/Lark via bot WebSocket.")
                .color(body_color)
                .size(label_size),
        );
        ui.add_space(styles.spacing.medium);

        // ── Onboarding wizard (inline) ───────────────────────────────────────
        if let Some(wiz) = onboarding
            && wiz.visible
        {
            ui.separator();
            ui.add_space(styles.spacing.medium);
            ui.label(
                egui::RichText::new("🧭 Channel Setup Wizard")
                    .color(heading_color)
                    .size(styles.typography.title5)
                    .strong(),
            );
            ui.add_space(styles.spacing.small);
            let profiles = profile_names.unwrap_or(&[]);
            let wiz_actions = wiz.draw(ui, styles, colors, profiles);
            actions.extend(wiz_actions);
            return actions;
        }

        // ── Empty state ──────────────────────────────────────────────────────
        if self.instances.is_empty() {
            ui.add_space(styles.spacing.medium);
            ui.label(
                egui::RichText::new(
                    "No channels configured. Click \"＋ Add Channel\" to get started.",
                )
                .color(body_color)
                .size(label_size),
            );
            return actions;
        }

        // ── Duplicate instance_id warning ────────────────────────────────────
        let dupes = self.duplicate_instance_ids();
        if !dupes.is_empty() {
            ui.colored_label(
                colors.danger,
                format!(
                    "⚠ Duplicate instance IDs detected: {}. This may cause config conflicts.",
                    dupes.join(", ")
                ),
            );
            ui.add_space(styles.spacing.small);
        }

        // ── Instance cards ───────────────────────────────────────────────────
        let field_width = ui
            .available_width()
            .min(styles.typography.line_height * 16.0);
        let field_height = styles.typography.line_height + styles.spacing.small;
        let store = self.secret_store.clone();
        let num_instances = self.instances.len();

        for idx in 0..num_instances {
            let inst = &self.instances[idx];
            let iid = inst.instance_id.clone();

            ui.separator();
            ui.add_space(styles.spacing.small);

            // ── Card header: collapse toggle + instance name + enable toggle + remove ──
            ui.horizontal(|ui| {
                let expanded = self.instances[idx].expanded;
                let collapse_icon = if expanded { "▼" } else { "▶" };
                if ui.button(collapse_icon).clicked() {
                    self.instances[idx].expanded = !expanded;
                }

                ui.label(
                    egui::RichText::new(&iid)
                        .color(heading_color)
                        .size(body_size)
                        .strong(),
                );

                ui.add_space(styles.spacing.medium);

                let prev_enabled = self.instances[idx].enabled;
                ui.checkbox(&mut self.instances[idx].enabled, "Enabled");
                if self.instances[idx].enabled != prev_enabled {
                    actions.push(UiAction::ToggleLarkChannelById {
                        instance_id: iid.clone(),
                        enabled: self.instances[idx].enabled,
                    });
                }

                // Connection status dot.
                let (dot_color, label_text) = match self.instances[idx].connection_state {
                    ConnectionState::Connected => (colors.success, "Connected"),
                    ConnectionState::Connecting => (colors.warning, "Connecting…"),
                    ConnectionState::Reconnecting => (colors.warning, "Reconnecting…"),
                    ConnectionState::Error => (colors.danger, "Error"),
                    ConnectionState::Disconnected => (colors.warning, "Disconnected"),
                };
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(
                        styles.spacing.medium + styles.spacing.small,
                        styles.spacing.medium + styles.spacing.small,
                    ),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .circle_filled(rect.center(), styles.radii.xs, dot_color);
                ui.label(
                    egui::RichText::new(label_text)
                        .color(body_color)
                        .size(label_size),
                );

                // Counters.
                ui.add_space(styles.spacing.small);
                ui.label(
                    egui::RichText::new(format!(
                        "↓ {} ↑ {}",
                        self.instances[idx].messages_received, self.instances[idx].messages_sent
                    ))
                    .color(body_color)
                    .size(label_size),
                );

                // Spacer.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("✕ Remove")
                                    .color(colors.danger)
                                    .size(label_size),
                            )
                            .fill(egui::Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        actions.push(UiAction::RemoveLarkChannel {
                            instance_id: iid.clone(),
                        });
                    }
                });
            });

            // Error message.
            if let Some(ref err) = self.instances[idx].last_error {
                ui.add_space(styles.spacing.medium);
                ui.colored_label(colors.danger, err);
            }

            // ── Collapsed: bail early for this instance ──────────────────────
            if !self.instances[idx].expanded {
                ui.add_space(styles.spacing.small);
                continue;
            }

            ui.add_space(styles.spacing.small);

            // Allowed users warning.
            if self.instances[idx].allowed_users_text.trim().is_empty() {
                ui.colored_label(
                colors.warning,
                "⚠ Allowed Users is empty — all inbound messages will be DENIED. Set to * to allow all.",
            );
                ui.add_space(styles.spacing.medium);
            }

            // ── Config form ──────────────────────────────────────────────────
            // App ID
            ui.label(
                egui::RichText::new("App ID")
                    .color(heading_color)
                    .size(label_size),
            );
            ui.add_sized(
                [field_width, field_height],
                egui::TextEdit::singleline(&mut self.instances[idx].app_id)
                    .id(egui::Id::new(("app_id", &iid)))
                    .hint_text("cli_xxxxx")
                    .text_color(heading_color),
            );
            ui.add_space(styles.spacing.small);

            // App Secret Env Var
            ui.label(
                egui::RichText::new("App Secret Env Variable")
                    .color(heading_color)
                    .size(label_size),
            );
            ui.add_sized(
                [field_width, field_height],
                egui::TextEdit::singleline(&mut self.instances[idx].app_secret_env)
                    .id(egui::Id::new(("app_secret_env", &iid)))
                    .hint_text("LARK_APP_SECRET")
                    .text_color(heading_color),
            );

            // Keychain controls for app secret
            if self.instances[idx].keychain_available && !self.instances[idx].app_id.is_empty() {
                let key_name =
                    crate::services::secret::channel_secret(&iid, &self.instances[idx].app_id);
                ui.add_space(styles.spacing.medium);
                ui.horizontal(|ui| {
                    if self.instances[idx].has_keychain_secret {
                        ui.label(
                            egui::RichText::new("🔐 Stored in keychain")
                                .color(body_color)
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
                        {
                            if let Err(e) = store.delete(&key_name) {
                                log::error!("Failed to delete keychain secret for {}: {}", iid, e);
                            }
                            self.instances[idx].has_keychain_secret = false;
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("App Secret:")
                                .color(body_color)
                                .size(label_size),
                        );
                        ui.add(
                            egui::TextEdit::singleline(
                                &mut self.instances[idx].keychain_secret_input,
                            )
                            .id(egui::Id::new(("keychain_input", &iid)))
                            .hint_text("Enter app secret...")
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
                            && !self.instances[idx].keychain_secret_input.is_empty()
                        {
                            if let Err(e) =
                                store.store(&key_name, &self.instances[idx].keychain_secret_input)
                            {
                                log::error!("Failed to store keychain secret for {}: {}", iid, e);
                            }
                            self.instances[idx].keychain_secret_input.clear();
                            self.instances[idx].has_keychain_secret = true;
                        }
                    }
                });
            }
            ui.add_space(styles.spacing.small);

            // Allowed Users
            ui.label(
                egui::RichText::new("Allowed Users (comma-separated open_ids, or * for all)")
                    .color(heading_color)
                    .size(label_size),
            );
            ui.add_sized(
                [field_width, field_height],
                egui::TextEdit::singleline(&mut self.instances[idx].allowed_users_text)
                    .id(egui::Id::new(("allowed_users", &iid)))
                    .hint_text("ou_xxxxx, ou_yyyyy")
                    .text_color(heading_color),
            );
            ui.add_space(styles.spacing.small);

            // API Base
            ui.label(
                egui::RichText::new("API Base URL")
                    .color(heading_color)
                    .size(label_size),
            );
            ui.add_sized(
                [field_width, field_height],
                egui::TextEdit::singleline(&mut self.instances[idx].api_base)
                    .id(egui::Id::new(("api_base", &iid)))
                    .hint_text("https://open.feishu.cn")
                    .text_color(heading_color),
            );
            ui.add_space(styles.spacing.small);

            // Profile dropdown
            ui.label(
                egui::RichText::new("Profile")
                    .color(heading_color)
                    .size(label_size),
            );
            if let Some(profiles) = profile_names {
                if !profiles.is_empty() {
                    let selected_name = profiles
                        .iter()
                        .find(|(id, _)| id == &self.instances[idx].profile_id)
                        .map(|(_, name)| name.as_str())
                        .unwrap_or("default");

                    egui::ComboBox::from_id_salt(format!("channel_profile_{}", iid))
                        .selected_text(selected_name)
                        .width(field_width - styles.spacing.medium)
                        .show_ui(ui, |ui| {
                            for (id, name) in profiles {
                                ui.selectable_value(
                                    &mut self.instances[idx].profile_id,
                                    id.clone(),
                                    name,
                                );
                            }
                        });
                } else {
                    ui.add_sized(
                        [field_width, field_height],
                        egui::TextEdit::singleline(&mut self.instances[idx].profile_id)
                            .id(egui::Id::new(("profile_id", &iid)))
                            .hint_text("default")
                            .text_color(heading_color),
                    );
                }
            } else {
                ui.add_sized(
                    [field_width, field_height],
                    egui::TextEdit::singleline(&mut self.instances[idx].profile_id)
                        .id(egui::Id::new(("profile_id_text", &iid)))
                        .hint_text("default")
                        .text_color(heading_color),
                );
            }
            ui.add_space(styles.spacing.medium);

            // ── Action buttons ───────────────────────────────────────────────
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    actions.push(UiAction::SaveChannelConfigById {
                        instance_id: iid.clone(),
                    });
                }

                ui.add_space(styles.spacing.small);

                let test_label = if self.instances[idx].testing {
                    "⏳ Testing…"
                } else {
                    "🔌 Test Connection"
                };
                let test_btn =
                    ui.add_enabled(!self.instances[idx].testing, egui::Button::new(test_label));
                if test_btn.clicked() {
                    self.instances[idx].testing = true;
                    actions.push(UiAction::TestChannelConnectionById {
                        instance_id: iid.clone(),
                    });
                }

                // Flash status message (save).
                if let Some((ref msg, saved_at)) = self.instances[idx].save_status {
                    let now = ui.ctx().input(|i| i.time);
                    if now - saved_at < 3.0 {
                        ui.add_space(styles.spacing.small);
                        let color = if msg.starts_with("Error") {
                            colors.danger
                        } else {
                            colors.success
                        };
                        ui.colored_label(color, msg);
                    } else {
                        self.instances[idx].save_status = None;
                    }
                }
            });

            // Test connection result.
            if let Some((ref msg, test_at)) = self.instances[idx].test_status {
                let now = ui.ctx().input(|i| i.time);
                if now - test_at < 8.0 {
                    ui.add_space(styles.spacing.medium);
                    let color = if msg.starts_with("✓") {
                        colors.success
                    } else {
                        colors.danger
                    };
                    ui.colored_label(color, msg);
                } else {
                    self.instances[idx].test_status = None;
                }
            }

            // Bot configuration check results (detailed diagnostic).
            if let Some(ref results) = self.instances[idx].bot_check_results {
                ui.add_space(styles.spacing.medium);
                ui.separator();
                ui.add_space(styles.spacing.medium);
                ui.label(egui::RichText::new("Bot Configuration Diagnostics").strong());
                ui.add_space(styles.spacing.medium);

                egui::Frame::new()
                    .fill(ui.visuals().window_fill.gamma_multiply(0.92))
                    .corner_radius(styles.radii.md)
                    .inner_margin(styles.spacing.medium)
                    .show(ui, |ui| {
                        for result in results {
                            let (icon, color) = if result.passed {
                                ("✓", colors.success)
                            } else {
                                ("✗", colors.danger)
                            };

                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(color, icon);
                                ui.label(egui::RichText::new(&result.label).color(color).strong());
                                ui.label(
                                    egui::RichText::new(&result.detail)
                                        .color(colors.text_caption)
                                        .small(),
                                );
                            });
                            ui.add_space(styles.spacing.small);
                        }
                    });
            }

            ui.add_space(styles.spacing.small);
        }

        actions
    }
}
