//! Onboarding Wizard — 4-step guided Lark channel setup.
//!
//! Steps:
//! 1. Login — authenticate with Lark (admin check)
//! 2. Create App — create or enter Lark bot app credentials
//! 3. Permissions — verify bot permissions (im:message, im:chat)
//! 4. Callback — configure webhook/WS endpoint
//!
//! Non-admin fallback: manual credential entry form (skips steps 1–4).
#![allow(dead_code)]

use crate::ui::actions::UiAction;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

// ─── Wizard Steps ────────────────────────────────────────────────────────────

/// Steps in the onboarding wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Login,
    CreateApp,
    Permissions,
    Callback,
    ManualEntry,
    Complete,
}

impl WizardStep {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Login => "1. Login",
            Self::CreateApp => "2. Create App",
            Self::Permissions => "3. Permissions",
            Self::Callback => "4. Callback",
            Self::ManualEntry => "Manual Entry",
            Self::Complete => "Complete",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "login" => Self::Login,
            "create_app" => Self::CreateApp,
            "permissions" => Self::Permissions,
            "callback" => Self::Callback,
            "manual" => Self::ManualEntry,
            "completed" => Self::Complete,
            _ => Self::Login,
        }
    }

    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::CreateApp => "create_app",
            Self::Permissions => "permissions",
            Self::Callback => "callback",
            Self::ManualEntry => "manual",
            Self::Complete => "completed",
        }
    }
}

// ─── Wizard State ────────────────────────────────────────────────────────────

/// Transient UI state for the onboarding wizard.
#[derive(Debug, Clone)]
pub struct OnboardingWizardState {
    /// Whether the wizard is currently visible.
    pub visible: bool,
    /// Current step in the wizard flow.
    pub current_step: WizardStep,
    /// Database row ID for persistence (if resuming).
    pub db_id: Option<i64>,
    /// Whether the user has admin privileges.
    pub is_admin: bool,

    // ── Input fields ─────────────────────────────────────────────────────
    /// Lark App ID entered by user.
    pub app_id: String,
    /// App secret entered during onboarding for keychain storage.
    pub app_secret: String,
    /// Environment variable name for app secret.
    pub app_secret_env: String,
    /// Whether to store the app secret in system keychain.
    pub store_secret_in_keychain: bool,
    /// API base URL.
    pub api_base: String,
    /// Allowed user IDs (comma-separated).
    pub allowed_users_text: String,
    /// Profile to bind to the channel.
    pub profile_id: String,

    // ── Status ───────────────────────────────────────────────────────────
    /// Status message for current step.
    pub status_message: Option<String>,
    /// Whether an async operation is in progress.
    pub loading: bool,
    /// Error from the last operation.
    pub error: Option<String>,
}

impl Default for OnboardingWizardState {
    fn default() -> Self {
        Self {
            visible: false,
            current_step: WizardStep::Login,
            db_id: None,
            is_admin: false,
            app_id: String::new(),
            app_secret: String::new(),
            app_secret_env: "LARK_APP_SECRET".into(),
            store_secret_in_keychain: true,
            api_base: "https://open.feishu.cn".into(),
            allowed_users_text: String::new(),
            profile_id: "default".into(),
            status_message: None,
            loading: false,
            error: None,
        }
    }
}

impl OnboardingWizardState {
    /// Restore wizard state from a database row.
    pub fn from_db(row: &crate::ai::db::OnboardingWizardRow) -> Self {
        Self {
            visible: true,
            current_step: WizardStep::from_db_str(&row.current_step),
            db_id: Some(row.id),
            is_admin: row.is_admin,
            app_id: row.lark_app_id.clone().unwrap_or_default(),
            ..Default::default()
        }
    }

    pub fn selected_secret_storage(&self) -> crate::services::secret::SecretStorage {
        if self.store_secret_in_keychain {
            crate::services::secret::SecretStorage::Keychain
        } else {
            crate::services::secret::SecretStorage::Env
        }
    }

    fn allowed_users(&self) -> Vec<String> {
        self.allowed_users_text
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

// ─── Wizard UI ───────────────────────────────────────────────────────────────

impl OnboardingWizardState {
    /// Draw the onboarding wizard overlay. Returns emitted `UiAction`s.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        profile_names: &[(String, String)], // (id, name) pairs
    ) -> Vec<UiAction> {
        let mut actions = Vec::new();

        if !self.visible {
            return actions;
        }

        let heading_color = colors.text_title;
        let body_color = colors.text_caption;

        // ── Step progress indicator ──────────────────────────────────────────
        ui.horizontal(|ui| {
            let steps = if self.is_admin {
                vec![
                    WizardStep::Login,
                    WizardStep::CreateApp,
                    WizardStep::Permissions,
                    WizardStep::Callback,
                ]
            } else {
                vec![WizardStep::ManualEntry]
            };

            for (i, step) in steps.iter().enumerate() {
                if i > 0 {
                    ui.label(egui::RichText::new(" → ").color(body_color));
                }
                let label = if *step == self.current_step {
                    egui::RichText::new(step.label())
                        .color(heading_color)
                        .strong()
                } else {
                    egui::RichText::new(step.label()).color(body_color)
                };
                ui.label(label);
            }
        });

        ui.add_space(styles.spacing.medium);

        // ── Step content ─────────────────────────────────────────────────────
        match self.current_step {
            WizardStep::Login => {
                self.draw_login_step(ui, styles, colors, &mut actions);
            }
            WizardStep::CreateApp => {
                self.draw_create_app_step(ui, styles, colors, &mut actions);
            }
            WizardStep::Permissions => {
                self.draw_permissions_step(ui, styles, colors, &mut actions);
            }
            WizardStep::Callback => {
                self.draw_callback_step(ui, styles, colors, profile_names, &mut actions);
            }
            WizardStep::ManualEntry => {
                self.draw_manual_entry_step(ui, styles, colors, profile_names, &mut actions);
            }
            WizardStep::Complete => {
                ui.label(
                    egui::RichText::new("✓ Onboarding complete! Channel configured.")
                        .color(heading_color),
                );
                if ui.button("Close").clicked() {
                    self.visible = false;
                }
            }
        }

        if self.loading || self.status_message.is_some() {
            ui.add_space(styles.spacing.medium);
            ui.horizontal(|ui| {
                if self.loading {
                    ui.spinner();
                }
                if let Some(status) = &self.status_message {
                    ui.label(egui::RichText::new(status).color(body_color));
                }
            });
        }

        // ── Error display ────────────────────────────────────────────────────
        if let Some(err) = &self.error {
            ui.add_space(styles.spacing.medium);
            ui.label(egui::RichText::new(format!("⚠ {}", err)).color(colors.danger));
        }

        actions
    }

    fn draw_login_step(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        let body_color = colors.text_caption;
        ui.label(
            egui::RichText::new("Log in to your Lark/Feishu account to begin setup.")
                .color(body_color),
        );
        ui.add_space(styles.spacing.medium);

        ui.horizontal(|ui| {
            if ui.button("I'm an Admin").clicked() {
                self.is_admin = true;
                self.current_step = WizardStep::CreateApp;
                actions.push(UiAction::OnboardingWizardStepChanged {
                    step: "create_app".into(),
                });
            }
            if ui.button("I'm not an Admin (Manual Entry)").clicked() {
                self.is_admin = false;
                self.current_step = WizardStep::ManualEntry;
                actions.push(UiAction::OnboardingWizardStepChanged {
                    step: "manual".into(),
                });
            }
        });
    }

    fn draw_create_app_step(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        let body_color = colors.text_caption;
        ui.label(
            egui::RichText::new(
                "Open the Lark Developer Console in the overlay, create or select your bot app, then capture its App ID and App Secret here.",
            )
            .color(body_color),
        );
        ui.add_space(styles.spacing.medium);

        ui.horizontal(|ui| {
            ui.label("App ID:");
            ui.text_edit_singleline(&mut self.app_id);
        });
        ui.horizontal(|ui| {
            ui.label("App Secret:");
            ui.add(egui::TextEdit::singleline(&mut self.app_secret).password(true));
        });
        ui.checkbox(
            &mut self.store_secret_in_keychain,
            "Store App Secret in system keychain",
        );
        if self.store_secret_in_keychain {
            ui.label(
                egui::RichText::new(
                    "Recommended: the secret will be stored under this Lark app in the system keychain.",
                )
                .color(body_color),
            );
        } else {
            ui.horizontal(|ui| {
                ui.label("App Secret Env Var:");
                ui.text_edit_singleline(&mut self.app_secret_env);
            });
        }

        ui.add_space(styles.spacing.medium);
        ui.horizontal(|ui| {
            if ui.button("Open Developer Console").clicked() {
                self.status_message =
                    Some("Opening the Lark Developer Console in the browser overlay…".into());
                self.error = None;
                actions.push(UiAction::OpenWebviewTab(
                    "https://open.feishu.cn/app".into(),
                ));
            }
            if ui.button("Auto-enable Bot + Batch Import").clicked() {
                self.error = None;
                actions.push(UiAction::OnboardingAutomateLarkSetup {
                    app_id: self.app_id.clone(),
                });
            }
        });

        ui.add_space(styles.spacing.medium);
        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.current_step = WizardStep::Login;
            }
            if ui.button("Next →").clicked() {
                if self.app_id.is_empty() || !self.app_id.starts_with("cli_") {
                    self.error = Some("App ID must start with 'cli_'".into());
                } else if self.store_secret_in_keychain && self.app_secret.trim().is_empty() {
                    self.error =
                        Some("Paste the App Secret so it can be stored in keychain.".into());
                } else if !self.store_secret_in_keychain && self.app_secret_env.trim().is_empty() {
                    self.error =
                        Some("App Secret env var is required when keychain storage is off.".into());
                } else {
                    self.error = None;
                    self.current_step = WizardStep::Permissions;
                    actions.push(UiAction::OnboardingWizardStepChanged {
                        step: "permissions".into(),
                    });
                }
            }
        });
    }

    fn draw_permissions_step(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        let body_color = colors.text_caption;
        ui.label(
            egui::RichText::new(
                "Confirm the bot and permissions page is ready. The overlay automation can enable the bot section and batch-import the recommended scopes.",
            )
            .color(body_color),
        );
        ui.add_space(styles.spacing.medium);
        ui.label("  • im:message — receive and send messages");
        ui.label("  • im:chat — read chat info for reply routing");
        ui.label("  • im:message.group_at_msg — receive @mentions in group chats");
        ui.add_space(styles.spacing.medium);
        if ui.button("Run Overlay Automation Again").clicked() {
            self.error = None;
            actions.push(UiAction::OnboardingAutomateLarkSetup {
                app_id: self.app_id.clone(),
            });
        }
        ui.add_space(styles.spacing.medium);

        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.current_step = WizardStep::CreateApp;
            }
            if ui.button("Permissions Verified → Next").clicked() {
                self.current_step = WizardStep::Callback;
                actions.push(UiAction::OnboardingWizardStepChanged {
                    step: "callback".into(),
                });
            }
        });
    }

    fn draw_callback_step(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        profile_names: &[(String, String)],
        actions: &mut Vec<UiAction>,
    ) {
        let body_color = colors.text_caption;
        ui.label(
            egui::RichText::new("Finalize the channel settings and store the credentials.")
                .color(body_color),
        );
        ui.add_space(styles.spacing.medium);
        ui.label("The bot connects via WebSocket, so no public callback URL is required.");
        if self.store_secret_in_keychain {
            ui.label(
                "The App Secret will be written to the system keychain when you complete setup.",
            );
        } else {
            ui.label("Ensure the App Secret env var is defined before testing the channel.");
        }

        ui.add_space(styles.spacing.medium);
        ui.horizontal(|ui| {
            ui.label("API Base:");
            ui.text_edit_singleline(&mut self.api_base);
        });
        ui.horizontal(|ui| {
            ui.label("Allowed Users (comma-separated):");
            ui.text_edit_singleline(&mut self.allowed_users_text);
        });

        if !profile_names.is_empty() {
            ui.add_space(styles.spacing.medium);
            ui.horizontal(|ui| {
                ui.label("Profile:");
                egui::ComboBox::from_id_salt("onboarding_profile_callback")
                    .selected_text(
                        profile_names
                            .iter()
                            .find(|(id, _)| id == &self.profile_id)
                            .map(|(_, name)| name.as_str())
                            .unwrap_or("default"),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in profile_names {
                            ui.selectable_value(&mut self.profile_id, id.clone(), name);
                        }
                    });
            });
        }

        ui.add_space(styles.spacing.medium);
        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.current_step = WizardStep::Permissions;
            }
            if ui.button("Complete Setup ✓").clicked() {
                if self.store_secret_in_keychain && self.app_secret.trim().is_empty() {
                    self.error =
                        Some("Paste the App Secret so it can be stored in keychain.".into());
                } else if !self.store_secret_in_keychain && self.app_secret_env.trim().is_empty() {
                    self.error =
                        Some("App Secret env var is required when keychain storage is off.".into());
                } else {
                    self.error = None;
                    self.loading = true;
                    self.status_message = Some("Saving the Lark channel configuration…".into());
                    actions.push(UiAction::OnboardingWizardComplete {
                        app_id: self.app_id.clone(),
                        app_secret: (!self.app_secret.trim().is_empty())
                            .then(|| self.app_secret.clone()),
                        app_secret_env: self.app_secret_env.clone(),
                        api_base: self.api_base.clone(),
                        allowed_users: self.allowed_users(),
                        profile_id: self.profile_id.clone(),
                        secret_storage: self.selected_secret_storage(),
                    });
                }
            }
        });
    }

    fn draw_manual_entry_step(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        profile_names: &[(String, String)],
        actions: &mut Vec<UiAction>,
    ) {
        let body_color = colors.text_caption;
        ui.label(
            egui::RichText::new("Enter your Lark bot credentials manually.").color(body_color),
        );
        ui.add_space(styles.spacing.medium);

        ui.horizontal(|ui| {
            ui.label("App ID:");
            ui.text_edit_singleline(&mut self.app_id);
        });
        ui.horizontal(|ui| {
            ui.label("App Secret:");
            ui.add(egui::TextEdit::singleline(&mut self.app_secret).password(true));
        });
        ui.checkbox(
            &mut self.store_secret_in_keychain,
            "Store App Secret in system keychain",
        );
        if !self.store_secret_in_keychain {
            ui.horizontal(|ui| {
                ui.label("App Secret Env Var:");
                ui.text_edit_singleline(&mut self.app_secret_env);
            });
        }
        ui.horizontal(|ui| {
            ui.label("API Base:");
            ui.text_edit_singleline(&mut self.api_base);
        });
        ui.horizontal(|ui| {
            ui.label("Allowed Users (comma-separated):");
            ui.text_edit_singleline(&mut self.allowed_users_text);
        });

        // Profile selector
        if !profile_names.is_empty() {
            ui.add_space(styles.spacing.medium);
            ui.horizontal(|ui| {
                ui.label("Profile:");
                egui::ComboBox::from_id_salt("onboarding_profile")
                    .selected_text(
                        profile_names
                            .iter()
                            .find(|(id, _)| id == &self.profile_id)
                            .map(|(_, name)| name.as_str())
                            .unwrap_or("default"),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in profile_names {
                            ui.selectable_value(&mut self.profile_id, id.clone(), name);
                        }
                    });
            });
        }

        ui.add_space(styles.spacing.medium);
        ui.horizontal(|ui| {
            if ui.button("← Back to Login").clicked() {
                self.current_step = WizardStep::Login;
            }
            if ui.button("Save & Complete ✓").clicked() {
                if self.app_id.is_empty() || !self.app_id.starts_with("cli_") {
                    self.error = Some("App ID must start with 'cli_'".into());
                } else if self.store_secret_in_keychain && self.app_secret.trim().is_empty() {
                    self.error =
                        Some("Paste the App Secret so it can be stored in keychain.".into());
                } else if !self.store_secret_in_keychain && self.app_secret_env.trim().is_empty() {
                    self.error =
                        Some("App secret env var is required when keychain storage is off.".into());
                } else {
                    self.error = None;
                    self.loading = true;
                    self.status_message = Some("Saving the Lark channel configuration…".into());
                    actions.push(UiAction::OnboardingWizardComplete {
                        app_id: self.app_id.clone(),
                        app_secret: (!self.app_secret.trim().is_empty())
                            .then(|| self.app_secret.clone()),
                        app_secret_env: self.app_secret_env.clone(),
                        api_base: self.api_base.clone(),
                        allowed_users: self.allowed_users(),
                        profile_id: self.profile_id.clone(),
                        secret_storage: self.selected_secret_storage(),
                    });
                }
            }
        });
    }
}
