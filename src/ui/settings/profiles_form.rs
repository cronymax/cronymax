use super::profiles::*;
use super::*;
use crate::profile::store::Profile;

impl ProfilesSettingsState {
    /// Draw the profile detail form with editable fields.
    pub(super) fn draw_form(
        &mut self,
        ui: &mut egui::Ui,
        profile_id: Option<&str>,
        config_path: Option<&std::path::Path>,
        styles: &Styles,
        colors: &Colors,
        _actions: &mut Vec<UiAction>,
    ) {
        let heading_color = colors.text_title;
        let body_color = colors.text_caption;
        let body_size = styles.typography.body2;
        let label_size = styles.typography.caption0;

        let title = match profile_id {
            Some(_id) => format!("Profile: {}", self.edit_name),
            None => "New Profile".to_string(),
        };
        ui.label(
            egui::RichText::new(title)
                .color(heading_color)
                .size(styles.typography.title3)
                .strong(),
        );

        // Show config file path for existing profiles.
        if let Some(path) = config_path {
            ui.label(
                egui::RichText::new(format!("Config: {}", path.display()))
                    .color(body_color)
                    .size(label_size),
            );
        }
        ui.add_space(styles.spacing.medium);

        ui.label(
            egui::RichText::new("── Sandbox Rules ──")
                .color(heading_color)
                .size(body_size)
                .strong(),
        );
        ui.add_space(styles.spacing.medium);

        // FS Read Allow
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("FS Read Allow:")
                    .color(body_color)
                    .size(label_size),
            );
            ui.text_edit_singleline(&mut self.edit_fs_read);
        });
        ui.add_space(styles.spacing.small);

        // FS Write Allow
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("FS Write Allow:")
                    .color(body_color)
                    .size(label_size),
            );
            ui.text_edit_singleline(&mut self.edit_fs_write);
        });
        ui.add_space(styles.spacing.small);

        // FS Deny
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("FS Deny:")
                    .color(body_color)
                    .size(label_size),
            );
            ui.text_edit_singleline(&mut self.edit_fs_deny);
        });
        ui.add_space(styles.spacing.small);

        // Network policy
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Network:")
                    .color(body_color)
                    .size(label_size),
            );
            let label = if self.edit_network_deny {
                "Deny all"
            } else {
                "Allow all"
            };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(label).size(label_size))
                        .fill(egui::Color32::TRANSPARENT),
                )
                .clicked()
            {
                self.edit_network_deny = !self.edit_network_deny;
            }
        });

        ui.add_space(styles.spacing.medium);
        ui.label(
            egui::RichText::new("── Skill Categories ──")
                .color(heading_color)
                .size(body_size)
                .strong(),
        );
        ui.add_space(styles.spacing.small);

        const ALL_CATEGORIES: &[&str] = &[
            "sandbox",
            "chat",
            "browser",
            "terminal",
            "tab",
            "webview",
            "external",
            "general",
            "channels",
            "scheduler",
        ];
        for &cat in ALL_CATEGORIES {
            let mut enabled = self.edit_allowed_skills.iter().any(|s| s == cat);
            if ui
                .checkbox(
                    &mut enabled,
                    egui::RichText::new(cat).size(label_size).color(body_color),
                )
                .changed()
            {
                if enabled {
                    if !self.edit_allowed_skills.iter().any(|s| s == cat) {
                        self.edit_allowed_skills.push(cat.to_string());
                    }
                } else {
                    self.edit_allowed_skills.retain(|s| s != cat);
                }
            }
        }
    }

    /// Load a profile's data into the editable state fields.
    pub(super) fn load_profile_into_state(&mut self, profile: &Profile) {
        self.edit_name = profile.name.clone();
        self.edit_allowed_skills = profile.allowed_skills.clone();
        // Load embedded sandbox if present; otherwise set defaults
        // (load_sandbox_into_state will override from policy.toml if available).
        if let Some(ref sandbox) = profile.sandbox {
            self.edit_fs_read = sandbox.fs.read_allow.join(", ");
            self.edit_fs_write = sandbox.fs.write_allow.join(", ");
            self.edit_fs_deny = sandbox.fs.deny.join(", ");
            self.edit_network_deny = sandbox.network.default_deny;
        } else {
            self.edit_fs_read = "~, /usr, /etc".into();
            self.edit_fs_write = "~, /tmp".into();
            self.edit_fs_deny = "~/.ssh, ~/.gnupg".into();
            self.edit_network_deny = false;
        }
    }

    /// Load sandbox policy from the profile directory into edit state fields.
    pub fn load_sandbox_into_state(
        &mut self,
        manager: &crate::profile::ProfileManager,
        profile_id: &str,
    ) {
        match manager.sandbox_policy(profile_id) {
            Ok(policy) => {
                self.edit_fs_read = policy.fs.read_allow.join(", ");
                self.edit_fs_write = policy.fs.write_allow.join(", ");
                self.edit_fs_deny = policy.fs.deny.join(", ");
                self.edit_network_deny = policy.network.default_deny;
            }
            Err(e) => {
                log::warn!("Failed to load sandbox policy for '{}': {}", profile_id, e);
            }
        }
    }

    /// Extract a SandboxPolicy from the current edit state.
    pub fn sandbox_policy_from_state(&self) -> crate::sandbox::policy::SandboxPolicy {
        use crate::sandbox::policy::{FsPolicy, NetworkPolicy, SandboxPolicy};
        let parse_csv = |s: &str| -> Vec<String> {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        };
        SandboxPolicy {
            fs: FsPolicy {
                read_allow: parse_csv(&self.edit_fs_read),
                write_allow: parse_csv(&self.edit_fs_write),
                deny: parse_csv(&self.edit_fs_deny),
            },
            network: NetworkPolicy {
                default_deny: self.edit_network_deny,
                allow_outbound: vec![],
                deny_outbound: vec![],
            },
        }
    }
}
