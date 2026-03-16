// Credential management skills — store, list, remove, resolve credentials via OS keychain.

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::services::secret::SecretStore;

// ─── Data Types ─────────────────────────────────────────────────────────────

/// A reference to a stored credential (never contains the secret value).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialEntry {
    pub service: String,
    pub key: String,
    pub created_at: String,
    pub last_accessed_at: String,
}

/// Top-level container for the credential registry file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialIndex {
    pub version: u32,
    pub entries: Vec<CredentialEntry>,
}

impl Default for CredentialIndex {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

// ─── Core Credential Functions ──────────────────────────────────────────────

/// Resolve the path to the credential index file.
fn index_path() -> PathBuf {
    crate::renderer::platform::config_dir().join("credentials.json")
}

/// Load the credential index from disk (or return default if missing/corrupt).
fn load_index() -> CredentialIndex {
    let path = index_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => CredentialIndex::default(),
    }
}

/// Save the credential index to disk.
fn save_index(index: &CredentialIndex) -> anyhow::Result<()> {
    let path = index_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(index)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Keyring key format: `{service}:{key}`.
fn keyring_key(service: &str, key: &str) -> String {
    format!("{service}:{key}")
}

/// Store a credential in the OS keychain and update the index.
pub fn credential_store(
    secret_store: &SecretStore,
    service: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    let kk = keyring_key(service, key);
    secret_store.store(&kk, value)?;

    let mut index = load_index();
    let now = Utc::now().to_rfc3339();

    if let Some(entry) = index
        .entries
        .iter_mut()
        .find(|e| e.service == service && e.key == key)
    {
        entry.last_accessed_at = now;
    } else {
        index.entries.push(CredentialEntry {
            service: service.to_string(),
            key: key.to_string(),
            created_at: now.clone(),
            last_accessed_at: now,
        });
    }
    save_index(&index)?;
    Ok(())
}

/// List all stored credential entries (names only, never values).
pub fn credential_list() -> Vec<CredentialEntry> {
    load_index().entries
}

/// Remove a credential from the OS keychain and the index.
pub fn credential_remove(
    secret_store: &SecretStore,
    service: &str,
    key: &str,
) -> anyhow::Result<()> {
    let kk = keyring_key(service, key);
    secret_store.delete(&kk)?;

    let mut index = load_index();
    index
        .entries
        .retain(|e| !(e.service == service && e.key == key));
    save_index(&index)?;
    Ok(())
}

/// Check whether a credential exists without revealing its value.
/// Auto-cleans stale index entries if the keychain returns None.
pub fn credential_resolve(
    secret_store: &SecretStore,
    service: &str,
    key: &str,
) -> anyhow::Result<bool> {
    let kk = keyring_key(service, key);
    let exists = secret_store
        .resolve(&kk, None, &crate::services::secret::SecretStorage::Keychain)?
        .is_some();

    if !exists {
        // Self-healing: remove stale index entry.
        let mut index = load_index();
        let before = index.entries.len();
        index
            .entries
            .retain(|e| !(e.service == service && e.key == key));
        if index.entries.len() != before {
            let _ = save_index(&index);
        }
    } else {
        // Update last_accessed_at.
        let mut index = load_index();
        if let Some(entry) = index
            .entries
            .iter_mut()
            .find(|e| e.service == service && e.key == key)
        {
            entry.last_accessed_at = Utc::now().to_rfc3339();
            let _ = save_index(&index);
        }
    }
    Ok(exists)
}

// ─── Skill Registration ─────────────────────────────────────────────────────

use std::sync::Arc;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};

/// Register all credential management skills.
pub fn register_credential_skills(registry: &mut SkillRegistry, secret_store: Arc<SecretStore>) {
    register_credential_store(registry, secret_store.clone());
    register_credential_list(registry);
    register_credential_remove(registry, secret_store.clone());
    register_credential_resolve(registry, secret_store);
}

fn register_credential_store(registry: &mut SkillRegistry, secret_store: Arc<SecretStore>) {
    let skill = Skill {
        name: "cronymax.credentials.store".into(),
        description: "Securely store a credential (API key, token, secret) in the system keychain."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "service": {
                    "type": "string",
                    "description": "Service name, e.g., 'openai', 'lark', 'anthropic'"
                },
                "key": {
                    "type": "string",
                    "description": "Key name within the service, e.g., 'api_key', 'app_secret'"
                },
                "value": {
                    "type": "string",
                    "description": "The secret value to store"
                }
            },
            "required": ["service", "key", "value"]
        }),
        category: "credentials".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let store = secret_store.clone();
        Box::pin(async move {
            let service = args["service"].as_str().unwrap_or("");
            let key = args["key"].as_str().unwrap_or("");
            let value = args["value"].as_str().unwrap_or("");
            if service.is_empty() || key.is_empty() || value.is_empty() {
                return Ok(json!({"error": "service, key, and value are required"}));
            }
            credential_store(&store, service, key, value)?;
            Ok(json!({"status": "stored", "service": service, "key": key}))
        })
    });
    registry.register(skill, handler);
}

fn register_credential_list(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.credentials.list".into(),
        description:
            "List all stored credentials by service and key name. Never returns secret values."
                .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
        category: "credentials".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        Box::pin(async move {
            let entries = credential_list();
            let items: Vec<Value> = entries
                .iter()
                .map(|e| json!({"service": e.service, "key": e.key, "created_at": e.created_at}))
                .collect();
            Ok(json!({"credentials": items, "count": items.len()}))
        })
    });
    registry.register(skill, handler);
}

fn register_credential_remove(registry: &mut SkillRegistry, secret_store: Arc<SecretStore>) {
    let skill = Skill {
        name: "cronymax.credentials.remove".into(),
        description: "Remove a stored credential from the system keychain.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "service": {
                    "type": "string",
                    "description": "Service name"
                },
                "key": {
                    "type": "string",
                    "description": "Key name"
                }
            },
            "required": ["service", "key"]
        }),
        category: "credentials".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let store = secret_store.clone();
        Box::pin(async move {
            let service = args["service"].as_str().unwrap_or("");
            let key = args["key"].as_str().unwrap_or("");
            if service.is_empty() || key.is_empty() {
                return Ok(json!({"error": "service and key are required"}));
            }
            credential_remove(&store, service, key)?;
            Ok(json!({"status": "removed", "service": service, "key": key}))
        })
    });
    registry.register(skill, handler);
}

fn register_credential_resolve(registry: &mut SkillRegistry, secret_store: Arc<SecretStore>) {
    let skill = Skill {
        name: "cronymax.credentials.resolve".into(),
        description:
            "Check if a credential exists in the system keychain without revealing its value."
                .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "service": {
                    "type": "string",
                    "description": "Service name"
                },
                "key": {
                    "type": "string",
                    "description": "Key name"
                }
            },
            "required": ["service", "key"]
        }),
        category: "credentials".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let store = secret_store.clone();
        Box::pin(async move {
            let service = args["service"].as_str().unwrap_or("");
            let key = args["key"].as_str().unwrap_or("");
            if service.is_empty() || key.is_empty() {
                return Ok(json!({"error": "service and key are required"}));
            }
            let exists = credential_resolve(&store, service, key)?;
            Ok(json!({"exists": exists, "service": service, "key": key}))
        })
    });
    registry.register(skill, handler);
}
