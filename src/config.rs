use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::ui::styles::colors::Colors;

/// Top-level application configuration loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct AppConfig {
    pub font: FontConfig,
    pub colors: ColorScheme,
    pub terminal: TerminalConfig,
    pub keybindings: Vec<KeyBinding>,
    pub webview: WebviewConfig,
    pub styles: crate::ui::styles::Styles,
    /// AI / LLM configuration (optional — existing configs without this still parse).
    pub ai: Option<AiConfig>,
    /// Profile management configuration (optional).
    pub profiles: Option<ProfilesConfig>,
    /// Claw mode (channels subsystem) configuration (optional).
    pub claw: Option<crate::channels::config::ClawConfig>,
    /// Skills subsystem configuration (optional).
    pub skills: Option<SkillsConfig>,
}

/// AI / LLM configuration in config.toml.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AiConfig {
    /// Maximum context window tokens.
    pub max_context_tokens: Option<usize>,
    /// Tokens reserved for completion output.
    pub reserve_tokens: Option<usize>,
    /// Default system prompt.
    pub system_prompt: Option<String>,
    /// Whether to auto-compact context when it reaches 80%.
    pub auto_compact: Option<bool>,
    /// Multiple LLM provider configurations.
    /// Each entry defines a provider endpoint that the model picker can query.
    pub providers: Option<Vec<ProviderConfig>>,
}

/// A single LLM provider endpoint configuration.
///
/// Example TOML:
/// ```toml
/// [[ai.providers]]
/// name = "OpenAI"
/// provider_type = "openai"
/// api_base = "https://api.openai.com/v1"
/// api_key_env = "OPENAI_API_KEY"
///
/// [[ai.providers]]
/// name = "Local Ollama"
/// provider_type = "ollama"
/// api_base = "http://localhost:11434/v1"
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Human-readable name for this provider endpoint.
    pub name: String,
    /// Provider type: "openai", "ollama", "anthropic", "copilot", "custom".
    pub provider_type: String,
    /// API base URL (overrides the default for the provider type).
    #[serde(default)]
    pub api_base: Option<String>,
    /// Environment variable name containing the API key.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Secret storage preference for the API key.
    #[serde(default)]
    pub secret_storage: crate::services::secret::SecretStorage,
}

/// Profile management configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProfilesConfig {
    /// Base directory for profiles (default: ~/.local/share/cronymax/profiles/).
    pub base_dir: Option<String>,
    /// Default profile ID to activate on startup.
    pub default_profile: Option<String>,
}

/// Skills subsystem configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct SkillsConfig {
    /// Base directory for installed skills (default: ~/.config/cronymax/skills/).
    pub skills_dir: Option<String>,
    /// ClawHub API base URL (default: https://clawhub.ai).
    pub clawhub_api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
    pub line_height: f32,
    pub bold_font: Option<String>,
    pub italic_font: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ColorScheme {
    pub foreground: String,
    pub background: String,
    pub cursor: String,
    pub selection_fg: String,
    pub selection_bg: String,
    pub ansi: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub shell: Option<String>,
    pub scrollback_lines: usize,
    pub cursor_style: String,
    pub cursor_blink: bool,
    pub default_mode: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct KeyBinding {
    pub key: String,
    pub modifiers: Vec<String>,
    pub action: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WebviewConfig {
    pub default_url: Option<String>,
    pub user_agent: Option<String>,
}

// ── Defaults ──────────────────────────────────────────────

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "monospace".into(),
            size: 14.0,
            line_height: 1.0,
            bold_font: None,
            italic_font: None,
        }
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            foreground: "#c0c0c0".into(),
            background: "#1e1e1e".into(),
            cursor: "#ffffff".into(),
            selection_fg: "#ffffff".into(),
            selection_bg: "#264f78".into(),
            ansi: vec![
                "#000000".into(),
                "#cd3131".into(),
                "#0dbc79".into(),
                "#e5e510".into(),
                "#2472c8".into(),
                "#bc3fbc".into(),
                "#11a8cd".into(),
                "#e5e5e5".into(),
                "#666666".into(),
                "#f14c4c".into(),
                "#23d18b".into(),
                "#f5f543".into(),
                "#3b8eea".into(),
                "#d670d6".into(),
                "#29b8db".into(),
                "#ffffff".into(),
            ],
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: None,
            scrollback_lines: 10_000,
            cursor_style: "block".into(),
            cursor_blink: false,
            default_mode: "editor".into(),
        }
    }
}

// ── Loading & Validation ─────────────────────────────────

/// Save just the `[[ai.providers]]` section to config.toml without clobbering
/// other user config. We do a read-modify-write on the TOML document.
pub fn save_providers_to_config(providers: &[ProviderConfig]) -> Result<(), String> {
    let path = AppConfig::config_path();
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = contents
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("Failed to parse config.toml: {}", e))?;

    // Ensure [ai] table exists.
    if doc.get("ai").is_none() {
        doc["ai"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    // Build the providers array of tables.
    let mut arr = toml_edit::ArrayOfTables::new();
    for p in providers {
        let mut tbl = toml_edit::Table::new();
        tbl["name"] = toml_edit::value(&p.name);
        tbl["provider_type"] = toml_edit::value(&p.provider_type);
        if let Some(ref base) = p.api_base
            && !base.is_empty()
        {
            tbl["api_base"] = toml_edit::value(base);
        }
        if let Some(ref key_env) = p.api_key_env
            && !key_env.is_empty()
        {
            tbl["api_key_env"] = toml_edit::value(key_env);
        }
        arr.push(tbl);
    }

    // Set ai.providers.
    if let Some(ai) = doc["ai"].as_table_mut() {
        if providers.is_empty() {
            ai.remove("providers");
        } else {
            ai.insert("providers", toml_edit::Item::ArrayOfTables(arr));
        }
    }

    // Write back.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("write: {}", e))?;
    Ok(())
}

/// Save `claw.enabled` to config.toml (read-modify-write).
pub fn save_claw_enabled(enabled: bool) -> Result<(), String> {
    let path = AppConfig::config_path();
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = contents
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("Failed to parse config.toml: {}", e))?;

    if doc.get("claw").is_none() {
        doc["claw"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    if let Some(claw) = doc["claw"].as_table_mut() {
        claw["enabled"] = toml_edit::value(enabled);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("write: {}", e))?;
    Ok(())
}

/// Save Lark channel configuration to config.toml (read-modify-write).
///
/// Writes both legacy `[claw.lark]` (backward compat) and `[[claw.channels]]`
/// (new format) so that `channels` Vec is populated on next load.
pub fn save_lark_config(lark: &crate::channels::config::LarkChannelConfig) -> Result<(), String> {
    let path = AppConfig::config_path();
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = contents
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("Failed to parse config.toml: {}", e))?;

    if doc.get("claw").is_none() {
        doc["claw"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    // Helper: build a TOML table from the Lark config.
    let build_lark_table = |with_type_field: bool| -> toml_edit::Table {
        let mut tbl = toml_edit::Table::new();
        if with_type_field {
            tbl["type"] = toml_edit::value("lark");
        }
        tbl["app_id"] = toml_edit::value(&lark.app_id);
        tbl["app_secret_env"] = toml_edit::value(&lark.app_secret_env);
        if lark.secret_storage != crate::services::secret::SecretStorage::Auto {
            tbl["secret_storage"] = toml_edit::value(match lark.secret_storage {
                crate::services::secret::SecretStorage::Auto => "auto",
                crate::services::secret::SecretStorage::Keychain => "keychain",
                crate::services::secret::SecretStorage::Env => "env",
            });
        }

        let mut users_arr = toml_edit::Array::new();
        for u in &lark.allowed_users {
            users_arr.push(u.as_str());
        }
        tbl["allowed_users"] = toml_edit::value(users_arr);

        if lark.api_base != "https://open.feishu.cn" {
            tbl["api_base"] = toml_edit::value(&lark.api_base);
        }
        tbl
    };

    if let Some(claw) = doc["claw"].as_table_mut() {
        // Write legacy [claw.lark] for backward compat.
        claw.insert("lark", toml_edit::Item::Table(build_lark_table(false)));

        // Write [[claw.channels]] array (replace existing entries).
        let mut channels_arr = toml_edit::ArrayOfTables::new();
        let lark_tbl = build_lark_table(true);
        channels_arr.push(lark_tbl);
        claw.insert("channels", toml_edit::Item::ArrayOfTables(channels_arr));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("write: {}", e))?;
    Ok(())
}

/// Save all channel configurations to config.toml (read-modify-write).
///
/// Writes `[[claw.channels]]` array with all instances and keeps legacy
/// `[claw.lark]` pointing at the first instance for backward compat.
pub fn save_channel_configs(
    channels: &[crate::channels::config::ChannelConfig],
) -> Result<(), String> {
    let path = AppConfig::config_path();
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = contents
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("Failed to parse config.toml: {}", e))?;

    if doc.get("claw").is_none() {
        doc["claw"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    if let Some(claw) = doc["claw"].as_table_mut() {
        // Build [[claw.channels]] array with all instances.
        let mut channels_arr = toml_edit::ArrayOfTables::new();
        for ch in channels {
            match ch {
                crate::channels::config::ChannelConfig::Lark(lark) => {
                    let mut tbl = toml_edit::Table::new();
                    tbl["type"] = toml_edit::value("lark");
                    tbl["instance_id"] = toml_edit::value(&lark.instance_id);
                    tbl["app_id"] = toml_edit::value(&lark.app_id);
                    tbl["app_secret_env"] = toml_edit::value(&lark.app_secret_env);
                    if lark.secret_storage != crate::services::secret::SecretStorage::Auto {
                        tbl["secret_storage"] = toml_edit::value(match lark.secret_storage {
                            crate::services::secret::SecretStorage::Auto => "auto",
                            crate::services::secret::SecretStorage::Keychain => "keychain",
                            crate::services::secret::SecretStorage::Env => "env",
                        });
                    }

                    let mut users_arr = toml_edit::Array::new();
                    for u in &lark.allowed_users {
                        users_arr.push(u.as_str());
                    }
                    tbl["allowed_users"] = toml_edit::value(users_arr);

                    if lark.api_base != "https://open.feishu.cn" {
                        tbl["api_base"] = toml_edit::value(&lark.api_base);
                    }
                    if lark.profile_id != "default" {
                        tbl["profile_id"] = toml_edit::value(&lark.profile_id);
                    }
                    channels_arr.push(tbl);
                }
            }
        }
        claw.insert("channels", toml_edit::Item::ArrayOfTables(channels_arr));

        // Keep legacy [claw.lark] pointing at the first Lark instance.
        if let Some(crate::channels::config::ChannelConfig::Lark(first)) = channels.first() {
            let mut legacy = toml_edit::Table::new();
            legacy["app_id"] = toml_edit::value(&first.app_id);
            legacy["app_secret_env"] = toml_edit::value(&first.app_secret_env);
            if first.secret_storage != crate::services::secret::SecretStorage::Auto {
                legacy["secret_storage"] = toml_edit::value(match first.secret_storage {
                    crate::services::secret::SecretStorage::Auto => "auto",
                    crate::services::secret::SecretStorage::Keychain => "keychain",
                    crate::services::secret::SecretStorage::Env => "env",
                });
            }
            let mut users_arr = toml_edit::Array::new();
            for u in &first.allowed_users {
                users_arr.push(u.as_str());
            }
            legacy["allowed_users"] = toml_edit::value(users_arr);
            if first.api_base != "https://open.feishu.cn" {
                legacy["api_base"] = toml_edit::value(&first.api_base);
            }
            claw.insert("lark", toml_edit::Item::Table(legacy));
        } else {
            claw.remove("lark");
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("write: {}", e))?;
    Ok(())
}

impl AppConfig {
    /// Load configuration from `~/.config/cronymax/config.toml`, falling back to defaults.
    pub fn load() -> Self {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                log::info!("Loading config from {}", path.display());
                match toml::from_str::<AppConfig>(&contents) {
                    Ok(mut cfg) => {
                        cfg.validate();
                        // Migrate legacy [claw.lark] → [[claw.channels]].
                        if let Some(ref mut claw) = cfg.claw {
                            claw.migrate_legacy();
                        }
                        cfg
                    }
                    Err(e) => {
                        log::warn!("Failed to parse config: {}. Using defaults.", e);
                        AppConfig::default()
                    }
                }
            }
            Err(_) => {
                log::info!("No config file at {}. Using defaults.", path.display());
                AppConfig::default()
            }
        }
    }

    /// Return the config file path: `~/.config/cronymax/config.toml`
    pub fn config_path() -> PathBuf {
        crate::renderer::platform::config_dir().join("config.toml")
    }

    /// Clamp values to valid ranges per config-schema contract.
    pub fn validate(&mut self) {
        self.font.size = self.font.size.clamp(1.0, 128.0);
        self.font.line_height = self.font.line_height.clamp(0.5, 3.0);
        self.terminal.scrollback_lines = self.terminal.scrollback_lines.min(1_000_000);

        if self.font.family.is_empty() {
            log::warn!("Empty font.family, falling back to 'monospace'");
            self.font.family = "monospace".into();
        }

        if self.colors.ansi.len() != 16 {
            log::warn!(
                "colors.ansi has {} entries (expected 16), using defaults",
                self.colors.ansi.len()
            );
            self.colors.ansi = ColorScheme::default().ansi;
        }

        match self.terminal.cursor_style.as_str() {
            "block" | "underline" | "beam" => {}
            other => {
                log::warn!("Unknown cursor_style '{}', defaulting to 'block'", other);
                self.terminal.cursor_style = "block".into();
            }
        }
    }

    /// Check if a dotted config key is "truthy" (used by skill gating).
    ///
    /// Supports: `claw.enabled`, `ai.auto_compact`, and other boolean-like
    /// fields accessible at runtime. Unknown keys return `false`.
    pub fn is_truthy(&self, key: &str) -> bool {
        match key {
            "claw.enabled" => self.claw.as_ref().map(|c| c.enabled).unwrap_or(false),
            "ai.auto_compact" => self
                .ai
                .as_ref()
                .and_then(|a| a.auto_compact)
                .unwrap_or(false),
            _ => {
                log::warn!("Unknown config key for is_truthy(): '{}'", key);
                false
            }
        }
    }
}

impl AppConfig {
    pub fn resolve_colors(&self) -> Colors {
        self.styles.colors_override().cloned().unwrap_or_default()
    }

    /// Convenience alias kept for call-sites that used the old name.
    pub fn resolve_egui_visuals(&self) -> egui::Visuals {
        self.styles.build_egui_visuals(&self.resolve_colors())
    }

    /// Build a complete egui::Style with proper spacing from design tokens.
    pub fn resolve_egui_style(&self) -> egui::Style {
        self.styles.build_egui_style(&self.resolve_colors())
    }
}
