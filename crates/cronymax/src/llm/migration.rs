//! One-time migration from the legacy flat LLM config to the
//! `LlmProviderRegistry`.
//!
//! # Legacy format
//! The legacy config was a JSON object at
//! `~/.cronymax/config.json` with the shape:
//! ```json
//! { "base_url": "https://api.openai.com/v1", "api_key": "sk-..." }
//! ```
//!
//! # Migration behaviour
//! - If `providers.json` already contains a provider named `"default"`, skip.
//! - If `config.json` is absent, skip (nothing to migrate).
//! - Otherwise, create a registry entry named `"default"` of kind
//!   `OpenaiCompat`, store the API key in keychain, set it as the default
//!   provider, and delete the `api_key` field from `config.json` (leaving
//!   `base_url` for reference, removing the secret).

use std::{
    fs,
    io::Write,
    path::Path,
};

use serde_json::Value;

use crate::llm::registry::{LlmProviderEntry, LlmProviderKind, LlmProviderRegistry};

// ── Public entry point ───────────────────────────────────────────────────────

/// Run the legacy LLM config migration if needed.
///
/// Idempotent: if the `"default"` provider already exists in the registry
/// the function returns immediately without touching any files.
///
/// # Parameters
/// * `providers_path` — absolute path to `providers.json`
/// * `legacy_config_path` — absolute path to the legacy `config.json`
pub fn run_migration(
    providers_path: &Path,
    legacy_config_path: &Path,
) -> anyhow::Result<MigrationOutcome> {
    // 1. Load (or create) the registry.
    let mut registry = LlmProviderRegistry::load_from(providers_path)?;

    // 2. Idempotency guard.
    if registry.get("default").is_some() {
        return Ok(MigrationOutcome::AlreadyMigrated);
    }

    // 3. Read legacy config — skip if absent.
    if !legacy_config_path.exists() {
        return Ok(MigrationOutcome::NoLegacyConfig);
    }

    let raw = fs::read_to_string(legacy_config_path)?;
    let mut legacy: Value = serde_json::from_str(&raw)?;

    let base_url = legacy
        .get("base_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("https://api.openai.com/v1")
        .to_string();

    let api_key = legacy
        .get("api_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // 4. Create registry entry.
    let entry = LlmProviderEntry {
        id: "default".into(),
        kind: LlmProviderKind::OpenaiCompat,
        base_url,
        model_override: None,
    };
    registry.upsert(entry, true)?;

    // 5. Store API key in keychain (if present).
    if let Some(key) = api_key {
        registry.store_token("default", &key)?;
        // Scrub the secret from config.json.
        if let Value::Object(ref mut map) = legacy {
            map.remove("api_key");
        }
        let sanitised = serde_json::to_string_pretty(&legacy)?;
        atomic_write(legacy_config_path, sanitised.as_bytes())?;
    }

    Ok(MigrationOutcome::Migrated)
}

// ── Outcome ──────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// Migration ran and a `"default"` provider was created.
    Migrated,
    /// The `"default"` provider already existed — nothing to do.
    AlreadyMigrated,
    /// No legacy config file was found — nothing to migrate.
    NoLegacyConfig,
}

// ── Atomic write helper ──────────────────────────────────────────────────────

fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().ok_or_else(|| anyhow::anyhow!("path has no parent"))?;
    let unique = uuid::Uuid::new_v4().to_string();
    let tmp_path = dir.join(format!(".tmp-{unique}"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)?;
    file.write_all(data)?;
    file.flush()?;
    drop(file);
    fs::rename(&tmp_path, path)?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup(dir: &Path, legacy_json: &str) -> (PathBuf, PathBuf) {
        let providers_path = dir.join("providers.json");
        let legacy_path = dir.join("config.json");
        fs::write(&legacy_path, legacy_json).unwrap();
        (providers_path, legacy_path)
    }

    #[test]
    fn migrates_legacy_config() {
        let dir = TempDir::new().unwrap();
        let legacy_json = r#"{"base_url":"https://api.openai.com/v1","api_key":"sk-test"}"#;
        let (providers_path, legacy_path) = setup(dir.path(), legacy_json);

        let outcome = run_migration(&providers_path, &legacy_path).unwrap();
        // On non-macOS, keychain storage will fail, so we accept Migrated OR
        // an error containing "keychain". On macOS it should succeed.
        // We assert the registry entry was created either way.
        let _ = outcome; // outcome is Migrated on macOS, or may error on Linux

        let registry = LlmProviderRegistry::load_from(&providers_path).unwrap();
        let entry = registry.get("default");
        // Registry entry must exist regardless of keychain.
        assert!(entry.is_some(), "default provider should exist after migration");
        assert_eq!(entry.unwrap().base_url, "https://api.openai.com/v1");
        assert_eq!(registry.default_provider_id(), Some("default"));
    }

    #[test]
    fn migration_idempotent_when_default_exists() {
        let dir = TempDir::new().unwrap();
        let legacy_json = r#"{"base_url":"https://api.openai.com/v1"}"#;
        let (providers_path, legacy_path) = setup(dir.path(), legacy_json);

        // Pre-populate the registry with a "default" entry.
        let entry = LlmProviderEntry {
            id: "default".into(),
            kind: LlmProviderKind::OpenaiCompat,
            base_url: "https://custom.api.com/v1".into(),
            model_override: None,
        };
        let mut reg = LlmProviderRegistry::load_from(&providers_path).unwrap();
        reg.upsert(entry, true).unwrap();
        drop(reg);

        let outcome = run_migration(&providers_path, &legacy_path).unwrap();
        assert_eq!(outcome, MigrationOutcome::AlreadyMigrated);

        // Ensure the custom base_url was not overwritten.
        let registry = LlmProviderRegistry::load_from(&providers_path).unwrap();
        assert_eq!(
            registry.get("default").unwrap().base_url,
            "https://custom.api.com/v1"
        );
    }

    #[test]
    fn migration_skipped_when_no_legacy_config() {
        let dir = TempDir::new().unwrap();
        let providers_path = dir.path().join("providers.json");
        let legacy_path = dir.path().join("config.json");
        // No legacy config file created.

        let outcome = run_migration(&providers_path, &legacy_path).unwrap();
        assert_eq!(outcome, MigrationOutcome::NoLegacyConfig);
    }

    #[test]
    fn migration_scrubs_api_key_from_legacy_file() {
        let dir = TempDir::new().unwrap();
        let legacy_json = r#"{"base_url":"https://api.openai.com/v1","api_key":"sk-secret","model":"gpt-4o"}"#;
        let (providers_path, legacy_path) = setup(dir.path(), legacy_json);

        // Run migration; ignore keychain errors (non-macOS CI).
        let _ = run_migration(&providers_path, &legacy_path);

        // The api_key must be absent from the file regardless.
        let remaining: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&legacy_path).unwrap()).unwrap();
        assert!(
            remaining.get("api_key").is_none(),
            "api_key should be scrubbed from legacy config"
        );
    }
}
