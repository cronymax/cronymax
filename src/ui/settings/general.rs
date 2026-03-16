//! Settings > General section — launch-on-startup toggle and app-level preferences.
#![allow(dead_code)]

use crate::ui::actions::UiAction;
use crate::ui::i18n::t;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// Transient state for the General settings section.
#[derive(Debug, Clone)]
pub struct GeneralSettingsState {
    /// Whether cronymax is registered to launch on login.
    pub launch_on_startup: bool,
    /// Whether Claw mode (channels subsystem) is enabled.
    pub claw_mode_enabled: bool,
    /// Whether there's been an error during the last toggle attempt.
    pub last_error: Option<String>,
}

impl GeneralSettingsState {
    /// Create a new state with the given Claw mode value from config.
    pub fn new(claw_enabled: bool) -> Self {
        Self {
            launch_on_startup: is_launch_on_startup_enabled(),
            claw_mode_enabled: claw_enabled,
            last_error: None,
        }
    }

    /// Draw the General section content.
    pub fn draw(&mut self, ui: &mut egui::Ui, styles: &Styles, colors: &Colors) -> Vec<UiAction> {
        let mut actions: Vec<UiAction> = Vec::new();

        let heading_color = colors.text_title;
        let body_color = colors.text_caption;
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let label_size = styles.typography.caption0;

        ui.label(
            egui::RichText::new(t("settings.general"))
                .color(heading_color)
                .size(heading_size)
                .strong(),
        );
        ui.add_space(styles.spacing.medium);

        // ── Launch on startup ────────────────────────────────────────────
        ui.horizontal(|ui| {
            let prev = self.launch_on_startup;
            ui.checkbox(&mut self.launch_on_startup, "");
            ui.label(
                egui::RichText::new(t("settings.launch_on_startup"))
                    .color(heading_color)
                    .size(body_size),
            );

            if self.launch_on_startup != prev {
                let result = if self.launch_on_startup {
                    enable_launch_on_startup()
                } else {
                    disable_launch_on_startup()
                };
                match result {
                    Ok(()) => {
                        self.last_error = None;
                        log::info!(
                            "Launch on startup {}",
                            if self.launch_on_startup {
                                "enabled"
                            } else {
                                "disabled"
                            }
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to toggle launch on startup: {}", e);
                        self.last_error = Some(e.to_string());
                        self.launch_on_startup = prev; // revert
                    }
                }
            }
        });
        ui.label(
            egui::RichText::new(t("settings.launch_on_startup_desc"))
                .color(body_color)
                .size(label_size),
        );

        if let Some(ref err) = self.last_error {
            ui.add_space(styles.spacing.medium);
            ui.colored_label(colors.danger, err);
        }

        ui.add_space(styles.spacing.large);

        // ── Enable Claw mode ─────────────────────────────────────────────
        ui.horizontal(|ui| {
            let prev = self.claw_mode_enabled;
            ui.checkbox(&mut self.claw_mode_enabled, "");
            ui.label(
                egui::RichText::new("Enable Claw mode")
                    .color(heading_color)
                    .size(body_size),
            );

            if self.claw_mode_enabled != prev {
                if self.claw_mode_enabled {
                    actions.push(UiAction::EnableClawMode);
                } else {
                    actions.push(UiAction::DisableClawMode);
                }
                log::info!(
                    "Claw mode {}",
                    if self.claw_mode_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
            }
        });
        ui.label(
            egui::RichText::new("Enable OpenClaw-like channels for bidirectional messaging with external platforms (e.g., Feishu/Lark).")
                .color(body_color)
                .size(label_size),
        );

        actions
    }
}

impl Default for GeneralSettingsState {
    fn default() -> Self {
        Self {
            launch_on_startup: is_launch_on_startup_enabled(),
            claw_mode_enabled: false,
            last_error: None,
        }
    }
}

// ─── Platform: macOS LaunchAgent ─────────────────────────────────────────────

#[cfg(target_os = "macos")]
const PLIST_LABEL: &str = "com.cronymax.app";

#[cfg(target_os = "macos")]
fn plist_path() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    home.join("Library/LaunchAgents")
        .join(format!("{}.plist", PLIST_LABEL))
}

#[cfg(target_os = "macos")]
fn is_launch_on_startup_enabled() -> bool {
    plist_path().exists()
}

#[cfg(target_os = "macos")]
fn enable_launch_on_startup() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>"#,
        PLIST_LABEL,
        exe.display()
    );

    let path = plist_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, plist_content)?;
    log::info!("Created LaunchAgent plist at {}", path.display());

    // Register with launchd so it takes effect immediately.
    let status = std::process::Command::new("launchctl")
        .arg("load")
        .arg("-w")
        .arg(&path)
        .status();
    match status {
        Ok(s) if s.success() => {
            log::info!("LaunchAgent loaded via launchctl");
        }
        Ok(s) => {
            log::warn!("launchctl load exited with status {}", s);
        }
        Err(e) => {
            log::warn!("Failed to run launchctl load: {}", e);
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn disable_launch_on_startup() -> anyhow::Result<()> {
    let path = plist_path();
    if path.exists() {
        // Unload from launchd first.
        let status = std::process::Command::new("launchctl")
            .arg("unload")
            .arg("-w")
            .arg(&path)
            .status();
        match status {
            Ok(s) if s.success() => {
                log::info!("LaunchAgent unloaded via launchctl");
            }
            Ok(s) => {
                log::warn!("launchctl unload exited with status {}", s);
            }
            Err(e) => {
                log::warn!("Failed to run launchctl unload: {}", e);
            }
        }
        std::fs::remove_file(&path)?;
        log::info!("Removed LaunchAgent plist at {}", path.display());
    }
    Ok(())
}

// ─── Platform: Linux (systemd user service / XDG autostart) ──────────────────

#[cfg(target_os = "linux")]
fn autostart_path() -> std::path::PathBuf {
    let config = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    config.join("autostart").join("cronymax.desktop")
}

#[cfg(target_os = "linux")]
fn is_launch_on_startup_enabled() -> bool {
    autostart_path().exists()
}

#[cfg(target_os = "linux")]
fn enable_launch_on_startup() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let desktop_entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=cronymax\n\
         Exec={}\n\
         X-GNOME-Autostart-enabled=true\n\
         Hidden=false\n",
        exe.display()
    );

    let path = autostart_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, desktop_entry)?;
    log::info!("Created XDG autostart entry at {}", path.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn disable_launch_on_startup() -> anyhow::Result<()> {
    let path = autostart_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        log::info!("Removed XDG autostart entry at {}", path.display());
    }
    Ok(())
}

// ─── Platform: Windows (registry Run key) ────────────────────────────────────

#[cfg(target_os = "windows")]
const REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const REG_VALUE: &str = "cronymax";

#[cfg(target_os = "windows")]
fn is_launch_on_startup_enabled() -> bool {
    use winreg::RegKey;
    use winreg::enums::*;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey(REG_KEY) {
        key.get_value::<String, _>(REG_VALUE).is_ok()
    } else {
        false
    }
}

#[cfg(target_os = "windows")]
fn enable_launch_on_startup() -> anyhow::Result<()> {
    use winreg::RegKey;
    use winreg::enums::*;
    let exe = std::env::current_exe()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_KEY)?;
    key.set_value(REG_VALUE, &exe.to_string_lossy().to_string())?;
    log::info!("Added Windows Run registry key for cronymax");
    Ok(())
}

#[cfg(target_os = "windows")]
fn disable_launch_on_startup() -> anyhow::Result<()> {
    use winreg::RegKey;
    use winreg::enums::*;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(REG_KEY, KEY_WRITE) {
        let _ = key.delete_value(REG_VALUE);
        log::info!("Removed Windows Run registry key for cronymax");
    }
    Ok(())
}
