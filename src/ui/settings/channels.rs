//! Settings > Channels section — channel configuration UI supporting typed ChannelConfig enum.
#![allow(dead_code)]

// Draw method is implemented in channels_draw.rs as `impl ChannelsSettingsState`.

use crate::channels::ConnectionState;
use crate::channels::config::{ChannelConfig, ClawConfig, LarkChannelConfig};

/// Per-instance UI state for a single Lark channel.
#[derive(Debug, Clone)]
pub struct LarkInstanceState {
    /// Unique instance identifier (e.g., "lark", "lark-2").
    pub instance_id: String,
    /// Whether this instance is enabled.
    pub enabled: bool,

    // ── Config fields (editable) ─────────────────────────────────────────
    pub app_id: String,
    pub app_secret_env: String,
    pub allowed_users_text: String,
    pub api_base: String,
    pub profile_id: String,

    // ── Live status ──────────────────────────────────────────────────────
    pub connection_state: ConnectionState,
    pub last_error: Option<String>,
    pub messages_received: u64,
    pub messages_sent: u64,

    // ── UI transient ─────────────────────────────────────────────────────
    pub save_status: Option<(String, std::time::Instant)>,
    pub test_status: Option<(String, std::time::Instant)>,
    pub testing: bool,
    pub bot_check_results: Option<Vec<crate::channels::BotCheckResult>>,
    pub keychain_available: bool,
    pub has_keychain_secret: bool,
    pub keychain_secret_input: String,
    /// Whether this instance's UI section is expanded.
    pub expanded: bool,
}

impl LarkInstanceState {
    /// Create a new empty instance with defaults.
    pub fn new_empty(
        instance_id: String,
        store: &std::sync::Arc<crate::services::secret::SecretStore>,
    ) -> Self {
        Self {
            instance_id,
            enabled: false,
            app_id: String::new(),
            app_secret_env: "LARK_APP_SECRET".to_string(),
            allowed_users_text: String::new(),
            api_base: "https://open.feishu.cn".to_string(),
            profile_id: "default".into(),
            connection_state: ConnectionState::Disconnected,
            last_error: None,
            messages_received: 0,
            messages_sent: 0,
            save_status: None,
            test_status: None,
            testing: false,
            bot_check_results: None,
            keychain_available: store.has_keychain(),
            has_keychain_secret: false,
            keychain_secret_input: String::new(),
            expanded: true,
        }
    }
}

/// Transient state for the Channels settings section.
#[derive(Debug, Clone)]
pub struct ChannelsSettingsState {
    /// All channel instance UI states.
    pub instances: Vec<LarkInstanceState>,
    /// Shared secret store (avoids repeated OS permission dialogs).
    pub secret_store: std::sync::Arc<crate::services::secret::SecretStore>,

    // ── Legacy single-instance fields (kept for backward compat during migration) ──
    /// Whether the Lark channel is enabled.
    pub lark_enabled: bool,
    pub app_id: String,
    pub app_secret_env: String,
    pub allowed_users_text: String,
    pub api_base: String,
    pub profile_id: String,
    pub connection_state: ConnectionState,
    pub last_error: Option<String>,
    pub messages_received: u64,
    pub messages_sent: u64,
    pub save_status: Option<(String, std::time::Instant)>,
    pub test_status: Option<(String, std::time::Instant)>,
    pub testing: bool,
    pub bot_check_results: Option<Vec<crate::channels::BotCheckResult>>,
    pub keychain_available: bool,
    pub has_keychain_secret: bool,
    pub keychain_secret_input: String,
}

impl Default for ChannelsSettingsState {
    fn default() -> Self {
        let store = std::sync::Arc::new(crate::services::secret::SecretStore::default());
        Self {
            instances: Vec::new(),
            secret_store: store.clone(),
            lark_enabled: false,
            app_id: String::new(),
            app_secret_env: "LARK_APP_SECRET".to_string(),
            allowed_users_text: String::new(),
            api_base: "https://open.feishu.cn".to_string(),
            profile_id: "default".into(),
            connection_state: ConnectionState::Disconnected,
            last_error: None,
            messages_received: 0,
            messages_sent: 0,
            save_status: None,
            test_status: None,
            testing: false,
            bot_check_results: None,
            keychain_available: store.has_keychain(),
            has_keychain_secret: false,
            keychain_secret_input: String::new(),
        }
    }
}

impl ChannelsSettingsState {
    /// Create with a shared secret store (avoids extra OS keychain probes).
    fn base_with_store(store: std::sync::Arc<crate::services::secret::SecretStore>) -> Self {
        let keychain_available = store.has_keychain();
        Self {
            instances: Vec::new(),
            secret_store: store.clone(),
            lark_enabled: false,
            app_id: String::new(),
            app_secret_env: "LARK_APP_SECRET".to_string(),
            allowed_users_text: String::new(),
            api_base: "https://open.feishu.cn".to_string(),
            profile_id: "default".into(),
            connection_state: ConnectionState::Disconnected,
            last_error: None,
            messages_received: 0,
            messages_sent: 0,
            save_status: None,
            test_status: None,
            testing: false,
            bot_check_results: None,
            keychain_available,
            has_keychain_secret: false,
            keychain_secret_input: String::new(),
        }
    }

    /// Initialize from an existing config. Reads the first Lark channel if present,
    /// or falls back to legacy `lark` field for backward compatibility.
    pub fn from_config(lark_cfg: Option<&LarkChannelConfig>, enabled: bool) -> Self {
        Self::from_config_with_store(
            lark_cfg,
            enabled,
            std::sync::Arc::new(crate::services::secret::SecretStore::default()),
        )
    }

    /// Like `from_config`, but reuses a shared secret store.
    pub fn from_config_with_store(
        lark_cfg: Option<&LarkChannelConfig>,
        enabled: bool,
        store: std::sync::Arc<crate::services::secret::SecretStore>,
    ) -> Self {
        let base = Self::base_with_store(store);
        match lark_cfg {
            Some(cfg) => {
                let key = crate::services::secret::channel_secret("lark", &cfg.app_id);
                let has_keychain = base
                    .secret_store
                    .resolve(
                        &key,
                        None,
                        &crate::services::secret::SecretStorage::Keychain,
                    )
                    .ok()
                    .flatten()
                    .is_some();
                Self {
                    lark_enabled: enabled,
                    app_id: cfg.app_id.clone(),
                    app_secret_env: cfg.app_secret_env.clone(),
                    allowed_users_text: cfg.allowed_users.join(", "),
                    api_base: cfg.api_base.clone(),
                    profile_id: cfg.profile_id.clone(),
                    has_keychain_secret: has_keychain,
                    ..base
                }
            }
            None => Self {
                lark_enabled: enabled,
                ..base
            },
        }
    }

    /// Initialize from a `ClawConfig`, reading from `channels` Vec first.
    pub fn from_claw_config(claw: Option<&ClawConfig>) -> Self {
        Self::from_claw_config_with_store(
            claw,
            std::sync::Arc::new(crate::services::secret::SecretStore::default()),
        )
    }

    /// Like `from_claw_config`, but reuses a shared secret store.
    /// Populates the `instances` Vec from all channels in the config.
    pub fn from_claw_config_with_store(
        claw: Option<&ClawConfig>,
        store: std::sync::Arc<crate::services::secret::SecretStore>,
    ) -> Self {
        let mut base = Self::base_with_store(store.clone());
        let claw = match claw {
            Some(c) => c,
            None => return base,
        };
        // Build per-instance state from all channels (and legacy lark field).
        let mut instances = Self::instances_from_claw_config(Some(claw), &store);
        // If no channels but legacy lark field exists, import it as an instance.
        if instances.is_empty()
            && let Some(ref legacy) = claw.lark
        {
            let key = crate::services::secret::channel_secret("lark", &legacy.app_id);
            let has_keychain = store
                .resolve(
                    &key,
                    None,
                    &crate::services::secret::SecretStorage::Keychain,
                )
                .ok()
                .flatten()
                .is_some();
            instances.push(LarkInstanceState {
                instance_id: legacy.instance_id.clone(),
                enabled: claw.enabled,
                app_id: legacy.app_id.clone(),
                app_secret_env: legacy.app_secret_env.clone(),
                allowed_users_text: legacy.allowed_users.join(", "),
                api_base: legacy.api_base.clone(),
                profile_id: legacy.profile_id.clone(),
                connection_state: ConnectionState::Disconnected,
                last_error: None,
                messages_received: 0,
                messages_sent: 0,
                save_status: None,
                test_status: None,
                testing: false,
                bot_check_results: None,
                keychain_available: store.has_keychain(),
                has_keychain_secret: has_keychain,
                keychain_secret_input: String::new(),
                expanded: false,
            });
        }
        // Also mirror the first instance into legacy flat fields for backward compat.
        if let Some(first) = instances.first() {
            base.lark_enabled = first.enabled;
            base.app_id = first.app_id.clone();
            base.app_secret_env = first.app_secret_env.clone();
            base.allowed_users_text = first.allowed_users_text.clone();
            base.api_base = first.api_base.clone();
            base.profile_id = first.profile_id.clone();
            base.has_keychain_secret = first.has_keychain_secret;
        }
        base.instances = instances;
        base
    }

    /// Convert the current UI state back into a `Vec<ChannelConfig>` for persistence.
    /// Preferred over `to_channel_config`/`to_lark_config` — serializes all instances.
    pub fn to_channel_configs(&self) -> Vec<ChannelConfig> {
        Self::instances_to_channel_configs(&self.instances)
    }

    /// Convert the current UI state back into a single `ChannelConfig::Lark`.
    /// Deprecated: prefer `to_channel_configs` for multi-instance support.
    pub fn to_channel_config(&self) -> ChannelConfig {
        let allowed_users: Vec<String> = self
            .allowed_users_text
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        ChannelConfig::Lark(LarkChannelConfig {
            instance_id: "lark".into(),
            app_id: self.app_id.clone(),
            app_secret_env: self.app_secret_env.clone(),
            allowed_users,
            api_base: self.api_base.clone(),
            profile_id: self.profile_id.clone(),
            secret_storage: Default::default(),
        })
    }

    /// Backward-compatible: convert to LarkChannelConfig.
    pub fn to_lark_config(&self) -> LarkChannelConfig {
        let allowed_users: Vec<String> = self
            .allowed_users_text
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        LarkChannelConfig {
            instance_id: "lark".into(),
            app_id: self.app_id.clone(),
            app_secret_env: self.app_secret_env.clone(),
            allowed_users,
            api_base: self.api_base.clone(),
            profile_id: self.profile_id.clone(),
            secret_storage: Default::default(),
        }
    }

    // ── Instance helpers (T005) ──────────────────────────────────────────

    /// Check for duplicate instance IDs. Returns IDs that appear more than once.
    pub fn duplicate_instance_ids(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut dupes = Vec::new();
        for inst in &self.instances {
            if !seen.insert(&inst.instance_id) && !dupes.contains(&inst.instance_id) {
                dupes.push(inst.instance_id.clone());
            }
        }
        dupes
    }

    /// Generate the next available instance ID (e.g. "lark", "lark-2", "lark-3").
    pub fn next_instance_id(&self) -> String {
        if !self.instances.iter().any(|i| i.instance_id == "lark") {
            return "lark".into();
        }
        let mut n = 2u32;
        loop {
            let candidate = format!("lark-{}", n);
            if !self.instances.iter().any(|i| i.instance_id == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// Find a mutable reference to an instance by ID.
    pub fn instance_mut(&mut self, instance_id: &str) -> Option<&mut LarkInstanceState> {
        self.instances
            .iter_mut()
            .find(|i| i.instance_id == instance_id)
    }

    /// Find an instance by ID (immutable).
    pub fn instance(&self, instance_id: &str) -> Option<&LarkInstanceState> {
        self.instances.iter().find(|i| i.instance_id == instance_id)
    }

    /// Remove an instance by ID. Returns `true` if removed.
    pub fn remove_instance(&mut self, instance_id: &str) -> bool {
        let before = self.instances.len();
        self.instances.retain(|i| i.instance_id != instance_id);
        self.instances.len() < before
    }

    // ── Config sync helpers (T006) ───────────────────────────────────────

    /// Build `Vec<LarkInstanceState>` from a `ClawConfig`.
    pub fn instances_from_claw_config(
        claw: Option<&ClawConfig>,
        store: &std::sync::Arc<crate::services::secret::SecretStore>,
    ) -> Vec<LarkInstanceState> {
        let claw = match claw {
            Some(c) => c,
            None => return Vec::new(),
        };
        claw.channels
            .iter()
            .map(|ch| match ch {
                ChannelConfig::Lark(cfg) => {
                    let key =
                        crate::services::secret::channel_secret(&cfg.instance_id, &cfg.app_id);
                    let has_keychain = store
                        .resolve(
                            &key,
                            None,
                            &crate::services::secret::SecretStorage::Keychain,
                        )
                        .ok()
                        .flatten()
                        .is_some();
                    LarkInstanceState {
                        instance_id: cfg.instance_id.clone(),
                        enabled: claw.enabled,
                        app_id: cfg.app_id.clone(),
                        app_secret_env: cfg.app_secret_env.clone(),
                        allowed_users_text: cfg.allowed_users.join(", "),
                        api_base: cfg.api_base.clone(),
                        profile_id: cfg.profile_id.clone(),
                        connection_state: ConnectionState::Disconnected,
                        last_error: None,
                        messages_received: 0,
                        messages_sent: 0,
                        save_status: None,
                        test_status: None,
                        testing: false,
                        bot_check_results: None,
                        keychain_available: store.has_keychain(),
                        has_keychain_secret: has_keychain,
                        keychain_secret_input: String::new(),
                        expanded: false,
                    }
                }
            })
            .collect()
    }

    /// Convert all instances back into `Vec<ChannelConfig>` for config persistence.
    pub fn instances_to_channel_configs(instances: &[LarkInstanceState]) -> Vec<ChannelConfig> {
        instances
            .iter()
            .map(|inst| {
                let allowed_users: Vec<String> = inst
                    .allowed_users_text
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                ChannelConfig::Lark(LarkChannelConfig {
                    instance_id: inst.instance_id.clone(),
                    app_id: inst.app_id.clone(),
                    app_secret_env: inst.app_secret_env.clone(),
                    allowed_users,
                    api_base: inst.api_base.clone(),
                    profile_id: inst.profile_id.clone(),
                    secret_storage: Default::default(),
                })
            })
            .collect()
    }
}
