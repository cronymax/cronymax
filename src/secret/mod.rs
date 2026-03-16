//! System keychain secret storage — cross-platform credential management.
//!
//! Uses the OS-native keychain (macOS Keychain, Windows Credential Manager,
//! Linux Secret Service) via the `keyring` crate to store and retrieve secrets
//! such as LLM API keys, channel app secrets, and OAuth tokens.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Default keyring service name (reverse-domain notation).
pub const KEYRING_SERVICE: &str = "com.cronymax.app";

/// Secret storage preference for a config entry.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SecretStorage {
    /// Try keychain first, fall back to env var.
    #[default]
    Auto,
    /// Only use system keychain. Fail if not available.
    Keychain,
    /// Only use environment variable (legacy behavior).
    Env,
}

/// Cross-platform secret storage backed by the OS keychain.
///
/// Caches keychain lookups in memory to avoid repeated OS permission dialogs
/// (macOS prompts the user each time the keychain is accessed by a new process).
#[derive(Clone, Debug)]
pub struct SecretStore {
    service: String,
    /// In-memory cache: `key → Some(value)` if found, `None` if confirmed absent.
    cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl SecretStore {
    /// Create a new SecretStore with a custom service name (useful for tests).
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Read a key from the keychain, using the in-memory cache when available.
    fn keychain_get(&self, key: &str) -> Result<Option<String>, keyring::Error> {
        // Check cache first.
        if let Ok(cache) = self.cache.lock()
            && let Some(cached) = cache.get(key)
        {
            return Ok(cached.clone());
        }
        // Cache miss — hit the real keychain.
        let entry = keyring::Entry::new(&self.service, key)?;
        let result = match entry.get_password() {
            Ok(val) => Some(val),
            Err(keyring::Error::NoEntry) => None,
            Err(e) => return Err(e),
        };
        // Populate cache.
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), result.clone());
        }
        Ok(result)
    }

    /// Resolve a secret value using the specified storage mode.
    ///
    /// Resolution priority for `Auto`:
    ///   1. System keychain (`keyring::Entry`)
    ///   2. Environment variable (if `env_var` is Some)
    ///
    /// Returns `Ok(Some(value))` if found, `Ok(None)` if not configured,
    /// or `Err(...)` for keychain errors in `Keychain`-only mode.
    pub fn resolve(
        &self,
        key: &str,
        env_var: Option<&str>,
        mode: &SecretStorage,
    ) -> anyhow::Result<Option<String>> {
        match mode {
            SecretStorage::Env => {
                // Legacy: only check env var.
                match env_var.and_then(|v| std::env::var(v).ok()) {
                    Some(val) => {
                        log::debug!("Secret '{}': resolved from env var", key);
                        Ok(Some(val))
                    }
                    None => {
                        log::debug!("Secret '{}': not found in env vars", key);
                        Ok(None)
                    }
                }
            }
            SecretStorage::Keychain => {
                // Strict keychain: error if unavailable.
                match self.keychain_get(key) {
                    Ok(Some(val)) => {
                        log::debug!("Secret '{}': resolved from keychain (strict mode)", key);
                        Ok(Some(val))
                    }
                    Ok(None) => {
                        log::debug!("Secret '{}': not found in keychain (strict mode)", key);
                        Ok(None)
                    }
                    Err(e) => Err(anyhow::anyhow!("Keychain error for '{}': {}", key, e)),
                }
            }
            SecretStorage::Auto => {
                // Try keychain first, fall back to env var.
                match self.keychain_get(key) {
                    Ok(Some(val)) => {
                        log::debug!("Secret '{}': resolved from keychain", key);
                        return Ok(Some(val));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::warn!(
                            "Keychain lookup failed for '{}': {} — falling back to env var",
                            key,
                            e
                        );
                    }
                }
                // Fallback to env var.
                if let Some(var_name) = env_var
                    && let Ok(val) = std::env::var(var_name)
                {
                    log::debug!("Secret '{}': resolved from env var ${}", key, var_name);
                    return Ok(Some(val));
                }
                log::debug!("Secret '{}': not found in keychain or env vars", key);
                Ok(None)
            }
        }
    }

    /// Store a secret in the system keychain.
    pub fn store(&self, key: &str, value: &str) -> anyhow::Result<()> {
        // If the in-memory cache already holds the same value, skip the keychain
        // write entirely.  On macOS each `set_password` to a new/modified keychain
        // item can trigger a system permission dialog, so avoiding redundant writes
        // reduces the number of prompts the user sees.
        if let Ok(cache) = self.cache.lock()
            && let Some(Some(existing)) = cache.get(key)
            && existing == value
        {
            return Ok(());
        }
        let entry = keyring::Entry::new(&self.service, key)?;
        entry
            .set_password(value)
            .map_err(|e| anyhow::anyhow!("Failed to store secret '{}': {}", key, e))?;
        // Update cache.
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), Some(value.to_string()));
        }
        Ok(())
    }

    /// Delete a secret from the system keychain. Idempotent — no error if absent.
    pub fn delete(&self, key: &str) -> anyhow::Result<()> {
        let entry = keyring::Entry::new(&self.service, key)?;
        match entry.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {} // already absent
            Err(e) => return Err(anyhow::anyhow!("Failed to delete secret '{}': {}", key, e)),
        }
        // Update cache.
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), None);
        }
        Ok(())
    }

    /// Check if the system keychain is available on this platform.
    pub fn has_keychain(&self) -> bool {
        // Use cache — the probe result won't change within a session.
        if let Ok(cache) = self.cache.lock()
            && cache.contains_key("__keychain_probe__")
        {
            return true; // We got a result (even None means keychain works).
        }
        // Try to create an entry — if the platform has no backend, this will fail.
        let entry = keyring::Entry::new(&self.service, "__keychain_probe__");
        match entry {
            Ok(e) => {
                // Try a get — NoEntry is fine (keychain works), other errors mean no keychain.
                let available = match e.get_password() {
                    Err(keyring::Error::NoEntry) => true,
                    Ok(_) => true,
                    Err(_) => false,
                };
                if available && let Ok(mut cache) = self.cache.lock() {
                    cache.insert("__keychain_probe__".to_string(), None);
                }
                available
            }
            Err(_) => false,
        }
    }
}

// ─── Key Name Builders ───────────────────────────────────────────────────────

/// Build keyring key for a provider API key.
/// Example: `provider_api_key("OpenAI")` → `"provider:openai:api-key"`
pub fn provider_api_key(provider_name: &str) -> String {
    format!("provider:{}:api-key", provider_name.to_lowercase())
}

/// Build keyring key for a channel secret.
/// Example: `channel_secret("lark", "cli_xxx")` → `"channel:lark:cli_xxx:secret"`
pub fn channel_secret(channel_type: &str, channel_id: &str) -> String {
    format!("channel:{}:{}:secret", channel_type, channel_id)
}

/// Build keyring key for an OAuth token.
/// Example: `oauth_token("lark")` → `"oauth:lark:token"`
pub fn oauth_token(provider: &str) -> String {
    format!("oauth:{}:token", provider)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_store() -> SecretStore {
        // Use the mock credential builder for tests — no real keychain access.
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::with_service("com.cronymax.test")
    }

    #[test]
    fn test_secret_storage_default() {
        let ss: SecretStorage = SecretStorage::default();
        assert_eq!(ss, SecretStorage::Auto);
    }

    #[test]
    fn test_secret_storage_serde_roundtrip() {
        let cases = [
            (SecretStorage::Auto, "\"auto\""),
            (SecretStorage::Keychain, "\"keychain\""),
            (SecretStorage::Env, "\"env\""),
        ];
        for (variant, expected_json) in &cases {
            let json = serde_json::to_string(variant).unwrap();
            assert_eq!(&json, expected_json);
            let parsed: SecretStorage = serde_json::from_str(expected_json).unwrap();
            assert_eq!(&parsed, variant);
        }
    }

    #[test]
    fn test_store_and_resolve_keychain() {
        let store = mock_store();
        let key = "provider:test:api-key";
        store.store(key, "sk-test-123").unwrap();

        let resolved = store.resolve(key, None, &SecretStorage::Auto).unwrap();
        assert_eq!(resolved, Some("sk-test-123".to_string()));
    }

    #[test]
    fn test_resolve_env_mode() {
        let store = mock_store();
        // Store in keychain — should NOT be found in Env mode.
        store
            .store("provider:test:api-key", "from-keychain")
            .unwrap();

        // Set env var for the test.
        unsafe { std::env::set_var("_CRONYMAX_TEST_KEY", "from-env") };
        let resolved = store
            .resolve(
                "provider:test:api-key",
                Some("_CRONYMAX_TEST_KEY"),
                &SecretStorage::Env,
            )
            .unwrap();
        assert_eq!(resolved, Some("from-env".to_string()));
        unsafe { std::env::remove_var("_CRONYMAX_TEST_KEY") };
    }

    #[test]
    fn test_resolve_auto_keychain_priority() {
        let store = mock_store();
        let key = "provider:test2:api-key";
        store.store(key, "from-keychain").unwrap();

        // Even with env var set, keychain should win in Auto mode.
        unsafe { std::env::set_var("_CRONYMAX_TEST_KEY2", "from-env") };
        let resolved = store
            .resolve(key, Some("_CRONYMAX_TEST_KEY2"), &SecretStorage::Auto)
            .unwrap();
        assert_eq!(resolved, Some("from-keychain".to_string()));
        unsafe { std::env::remove_var("_CRONYMAX_TEST_KEY2") };
    }

    #[test]
    fn test_resolve_auto_env_fallback() {
        let store = mock_store();
        let key = "provider:nonexistent:api-key";

        unsafe { std::env::set_var("_CRONYMAX_TEST_KEY3", "from-env-fallback") };
        let resolved = store
            .resolve(key, Some("_CRONYMAX_TEST_KEY3"), &SecretStorage::Auto)
            .unwrap();
        assert_eq!(resolved, Some("from-env-fallback".to_string()));
        unsafe { std::env::remove_var("_CRONYMAX_TEST_KEY3") };
    }

    #[test]
    fn test_resolve_not_found() {
        let store = mock_store();
        let resolved = store
            .resolve("provider:missing:api-key", None, &SecretStorage::Auto)
            .unwrap();
        assert_eq!(resolved, None);
    }

    #[test]
    fn test_delete_idempotent() {
        let store = mock_store();
        let key = "provider:delete-test:api-key";
        store.store(key, "value").unwrap();
        store.delete(key).unwrap();
        // Second delete should not error.
        store.delete(key).unwrap();

        let resolved = store.resolve(key, None, &SecretStorage::Auto).unwrap();
        assert_eq!(resolved, None);
    }

    #[test]
    fn test_key_builders() {
        assert_eq!(provider_api_key("OpenAI"), "provider:openai:api-key");
        assert_eq!(
            channel_secret("lark", "cli_xxx"),
            "channel:lark:cli_xxx:secret"
        );
        assert_eq!(oauth_token("lark"), "oauth:lark:token");
    }

    #[test]
    fn test_has_keychain() {
        let store = mock_store();
        // Mock backend should report as available.
        assert!(store.has_keychain());
    }
}
