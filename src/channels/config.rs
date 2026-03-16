//! Channel configuration types — ClawConfig, ChannelConfig, LarkChannelConfig.

use serde::{Deserialize, Serialize};

/// Application-level configuration for the channels subsystem.
///
/// Stored in `config.toml` under `[claw]`.
///
/// ```toml
/// [claw]
/// enabled = true
///
/// [[claw.channels]]
/// type = "lark"
/// app_id = "cli_xxxxx"
/// app_secret_env = "LARK_APP_SECRET"
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[derive(Default)]
pub struct ClawConfig {
    /// Master toggle for Claw mode. When `false`, no channels are initialized.
    pub enabled: bool,
    /// List of channel configurations. Each entry creates one channel instance.
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
    /// Legacy: Feishu/Lark channel configuration (deprecated, use `channels`).
    /// Kept for backward compatibility — migrated to `channels` on load.
    #[serde(skip_serializing)]
    pub lark: Option<LarkChannelConfig>,
}

impl ClawConfig {
    /// Migrate legacy `[claw.lark]` config to `[[claw.channels]]` format.
    /// Call this after deserialization to normalize the config.
    pub fn migrate_legacy(&mut self) {
        if let Some(lark) = self.lark.take() {
            // Only migrate if not already present in channels list.
            let already_has_lark = self
                .channels
                .iter()
                .any(|c| matches!(c, ChannelConfig::Lark(_)));
            if !already_has_lark {
                self.channels.push(ChannelConfig::Lark(lark));
            }
        }
    }

    /// Validate that all channel instance IDs are unique.
    pub fn validate_unique_ids(&self) -> anyhow::Result<()> {
        let mut seen = std::collections::HashSet::new();
        for ch in &self.channels {
            let id = ch.instance_id();
            if !seen.insert(id) {
                anyhow::bail!("Duplicate channel instance_id: '{}'", id);
            }
        }
        Ok(())
    }
}

/// Typed channel configuration enum.
///
/// Each variant holds the platform-specific config. Uses `#[serde(tag = "type")]`
/// for TOML/JSON serialization, so each entry has a `type` discriminator field.
///
/// ```toml
/// [[claw.channels]]
/// type = "lark"
/// app_id = "cli_xxxxx"
/// app_secret_env = "LARK_APP_SECRET"
/// ```
///
/// # Adding a New Channel
///
/// 1. Define `{Platform}ChannelConfig` struct
/// 2. Add a variant to this enum: `{Platform}({Platform}ChannelConfig)`
/// 3. Add a match arm in `register_channels()` (src/channel/mod.rs)
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ChannelConfig {
    /// Feishu/Lark channel.
    Lark(LarkChannelConfig),
}

impl ChannelConfig {
    /// Validate the channel configuration.
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            ChannelConfig::Lark(cfg) => cfg.validate(),
        }
    }

    /// Get a display name for this channel configuration.
    pub fn display_name(&self) -> &str {
        match self {
            ChannelConfig::Lark(_) => "Feishu/Lark",
        }
    }

    /// Get the instance ID for this channel configuration.
    pub fn instance_id(&self) -> &str {
        match self {
            ChannelConfig::Lark(cfg) => &cfg.instance_id,
        }
    }
}

/// Per-channel configuration for Feishu/Lark.
///
/// ```toml
/// [[claw.channels]]
/// type = "lark"
/// app_id = "cli_xxxxx"
/// app_secret_env = "LARK_APP_SECRET"
/// allowed_users = ["ou_xxxxx", "ou_yyyyy"]
/// api_base = "https://open.feishu.cn"
/// profile_id = "default"
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LarkChannelConfig {
    /// Unique instance identifier for multi-instance support.
    /// Defaults to "lark" if not specified.
    #[serde(default = "default_lark_instance_id")]
    pub instance_id: String,
    /// Feishu/Lark app ID (e.g., `cli_xxxxx`).
    pub app_id: String,
    /// Environment variable name containing the app secret.
    /// The raw secret is never stored in the config file.
    pub app_secret_env: String,
    /// List of Lark `open_id` values permitted to send messages.
    /// - Empty (`[]`) → deny all (default)
    /// - `["*"]` → allow all
    /// - Otherwise → exact-match per entry
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// API base URL. Defaults to `https://open.feishu.cn`.
    /// Use `https://open.larksuite.com` for international Lark.
    #[serde(default = "default_lark_api_base")]
    pub api_base: String,
    /// Profile ID to use for this channel. Controls skill allowlist and memory.
    /// Defaults to `"default"`.
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    /// Secret storage preference for the app secret.
    #[serde(default)]
    pub secret_storage: crate::services::secret::SecretStorage,
}

fn default_lark_api_base() -> String {
    "https://open.feishu.cn".to_string()
}

fn default_profile_id() -> String {
    "default".to_string()
}

fn default_lark_instance_id() -> String {
    "lark".to_string()
}

impl LarkChannelConfig {
    /// Validate the configuration, returning an error if invalid.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.app_id.is_empty() {
            anyhow::bail!("Lark app_id must not be empty");
        }
        if !self.app_id.starts_with("cli_") {
            anyhow::bail!("Lark app_id must start with 'cli_' (got '{}')", self.app_id);
        }
        // In keychain-only mode, app_secret_env can be empty.
        if self.secret_storage != crate::services::secret::SecretStorage::Keychain
            && self.app_secret_env.is_empty()
        {
            anyhow::bail!(
                "Lark app_secret_env must not be empty (or set secret_storage = \"keychain\")"
            );
        }
        if !self.api_base.starts_with("https://") {
            anyhow::bail!(
                "Lark api_base must be an HTTPS URL (got '{}')",
                self.api_base
            );
        }
        Ok(())
    }

    /// Resolve the app secret from keychain / environment variable.
    pub fn resolve_app_secret(
        &self,
        secret_store: &crate::services::secret::SecretStore,
    ) -> anyhow::Result<String> {
        let key = crate::services::secret::channel_secret("lark", &self.app_id);
        let env_var = if self.app_secret_env.is_empty() {
            None
        } else {
            Some(self.app_secret_env.as_str())
        };
        secret_store
            .resolve(&key, env_var, &self.secret_storage)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Lark app secret not found in keychain or env var '{}' for app_id '{}'",
                    self.app_secret_env,
                    self.app_id
                )
            })
    }
}
