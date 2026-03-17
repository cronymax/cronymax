//! Settings > LLM Providers section — manage multiple LLM provider endpoints.
#![allow(dead_code)]

use crate::config::ProviderConfig;

/// Known provider types for the dropdown.
pub(super) const PROVIDER_TYPES: &[&str] = &["openai", "ollama", "copilot", "anthropic", "custom"];

/// Returns `(default_name, default_api_base, default_api_key_env)` for a provider type.
pub(super) fn provider_defaults(provider_type: &str) -> (&'static str, &'static str, &'static str) {
    match provider_type {
        "openai" => ("OpenAI", "https://api.openai.com/v1", "OPENAI_API_KEY"),
        "anthropic" => (
            "Anthropic",
            "https://api.anthropic.com",
            "ANTHROPIC_API_KEY",
        ),
        "ollama" => ("Ollama", "http://localhost:11434", ""),
        "copilot" => ("Copilot", "https://api.githubcopilot.com", "GH_TOKEN"),
        _ => ("", "", ""),
    }
}

/// Transient state for the LLM Providers settings section.
#[derive(Debug, Clone)]
pub struct ProvidersSettingsState {
    /// Current list of configured providers (editable copy).
    pub providers: Vec<ProviderEntry>,
    /// Index of the provider currently being edited, or None.
    pub editing_index: Option<usize>,
    /// Whether we are adding a new provider (form visible).
    pub adding_new: bool,
    /// Editable fields for the new/editing provider.
    pub edit_name: String,
    pub edit_provider_type: String,
    pub edit_api_base: String,
    pub edit_api_key_env: String,
    /// Status message shown after save.
    pub save_status: Option<(String, f64)>,
    /// Whether state has been loaded from config.
    pub loaded: bool,
    /// Whether the system keychain is available on this platform.
    pub keychain_available: bool,
    /// Inline secret input for storing in keychain (transient, never persisted).
    pub keychain_secret_input: String,
    /// Shared secret store (avoids repeated OS permission dialogs).
    pub secret_store: std::sync::Arc<crate::services::secret::SecretStore>,
}

/// A single provider entry in the editable list.
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub name: String,
    pub provider_type: String,
    pub api_base: String,
    pub api_key_env: String,
    /// Whether this provider has a secret stored in the system keychain.
    pub has_keychain_secret: bool,
}

impl ProviderEntry {
    pub fn from_config(cfg: &ProviderConfig, store: &crate::services::secret::SecretStore) -> Self {
        let key = crate::services::secret::provider_api_key(&cfg.name);
        let has_keychain = store
            .resolve(
                &key,
                None,
                &crate::services::secret::SecretStorage::Keychain,
            )
            .ok()
            .flatten()
            .is_some();
        Self {
            name: cfg.name.clone(),
            provider_type: cfg.provider_type.clone(),
            api_base: cfg.api_base.clone().unwrap_or_default(),
            api_key_env: cfg.api_key_env.clone().unwrap_or_default(),
            has_keychain_secret: has_keychain,
        }
    }

    pub fn to_provider_config(&self) -> ProviderConfig {
        ProviderConfig {
            name: self.name.clone(),
            provider_type: self.provider_type.clone(),
            api_base: if self.api_base.is_empty() {
                None
            } else {
                Some(self.api_base.clone())
            },
            api_key_env: if self.api_key_env.is_empty() {
                None
            } else {
                Some(self.api_key_env.clone())
            },
            secret_storage: Default::default(),
        }
    }
}

impl Default for ProvidersSettingsState {
    fn default() -> Self {
        let store = std::sync::Arc::new(crate::services::secret::SecretStore::default());
        Self {
            providers: Vec::new(),
            editing_index: None,
            adding_new: false,
            edit_name: String::new(),
            edit_provider_type: "openai".into(),
            edit_api_base: String::new(),
            edit_api_key_env: String::new(),
            save_status: None,
            loaded: false,
            keychain_available: store.has_keychain(),
            keychain_secret_input: String::new(),
            secret_store: store,
        }
    }
}

impl ProvidersSettingsState {
    /// Create with a shared secret store (avoids extra OS keychain probes).
    pub fn with_secret_store(store: std::sync::Arc<crate::services::secret::SecretStore>) -> Self {
        let keychain_available = store.has_keychain();
        Self {
            providers: Vec::new(),
            editing_index: None,
            adding_new: false,
            edit_name: String::new(),
            edit_provider_type: "openai".into(),
            edit_api_base: String::new(),
            edit_api_key_env: String::new(),
            save_status: None,
            loaded: false,
            keychain_available,
            keychain_secret_input: String::new(),
            secret_store: store,
        }
    }

    /// Load providers from AppConfig into the settings state.
    /// Also auto-detects env vars and presets provider entries if none exist.
    pub fn load_from_config(&mut self, config: &crate::config::AppConfig) {
        if let Some(ref ai) = config.ai
            && let Some(ref providers) = ai.providers
        {
            self.providers = providers
                .iter()
                .map(|p| ProviderEntry::from_config(p, &self.secret_store))
                .collect();
        }

        // Auto-detect and preset providers from env vars if no providers configured yet.
        if self.providers.is_empty() {
            let mut presets = Vec::new();

            if std::env::var("OPENAI_API_KEY").is_ok() {
                presets.push(ProviderEntry {
                    name: "OpenAI".into(),
                    provider_type: "openai".into(),
                    api_base: "https://api.openai.com/v1".into(),
                    api_key_env: "OPENAI_API_KEY".into(),
                    has_keychain_secret: false,
                });
            }

            if std::env::var("GH_TOKEN").is_ok() {
                presets.push(ProviderEntry {
                    name: "GitHub Copilot".into(),
                    provider_type: "copilot".into(),
                    api_base: "https://models.inference.ai.azure.com".into(),
                    api_key_env: "GH_TOKEN".into(),
                    has_keychain_secret: false,
                });
            }

            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                presets.push(ProviderEntry {
                    name: "Anthropic".into(),
                    provider_type: "anthropic".into(),
                    api_base: "https://api.anthropic.com/v1".into(),
                    api_key_env: "ANTHROPIC_API_KEY".into(),
                    has_keychain_secret: false,
                });
            }

            if !presets.is_empty() {
                log::info!(
                    "Auto-detected {} LLM provider(s) from environment variables",
                    presets.len()
                );
                self.providers = presets;
            }
        }

        self.loaded = true;
    }

    /// Convert the UI state back to a list of ProviderConfig.
    pub fn to_provider_configs(&self) -> Vec<ProviderConfig> {
        self.providers
            .iter()
            .map(|e| e.to_provider_config())
            .collect()
    }
}
