//! Named multi-provider registry.
//!
//! Persists to `~/.cronymax/providers.json`. Keychain items are stored
//! under `cronymax-provider-<id>` so no secret ever touches the JSON file.
//!
//! # Wire format  (`providers.json`)
//! ```json
//! {
//!   "default_provider": "openai",
//!   "providers": [
//!     {
//!       "id": "openai",
//!       "kind": "openai_compat",
//!       "base_url": "https://api.openai.com/v1",
//!       "model_override": null
//!     },
//!     {
//!       "id": "copilot",
//!       "kind": "github_copilot",
//!       "base_url": "https://api.githubcopilot.com",
//!       "model_override": null
//!     }
//!   ]
//! }
//! ```

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

// ── LlmProviderKind ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    OpenaiCompat,
    GithubCopilot,
    None,
}

// ── LlmProviderEntry ─────────────────────────────────────────────────────────

/// A single provider configuration row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderEntry {
    pub id: String,
    pub kind: LlmProviderKind,
    pub base_url: String,
    /// If set, every call through this provider uses this model regardless
    /// of what the agent or request specifies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
}

// ── Wire format root ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default)]
struct ProviderFile {
    #[serde(default)]
    default_provider: Option<String>,
    #[serde(default)]
    providers: Vec<LlmProviderEntry>,
}

// ── LlmProviderRegistry ──────────────────────────────────────────────────────

/// In-memory registry loaded from (and persisted to) `providers.json`.
/// All mutation methods atomically rewrite the file; keychain items are
/// never serialised to disk.
#[derive(Debug)]
pub struct LlmProviderRegistry {
    /// Absolute path to `providers.json`.
    path: PathBuf,
    file: ProviderFile,
}

impl LlmProviderRegistry {
    /// Load (or create) the registry from `~/.cronymax/providers.json`.
    pub fn load_default() -> anyhow::Result<Self> {
        let home = dirs_home()?;
        let path = home.join(".cronymax").join("providers.json");
        Self::load_from(&path)
    }

    /// Load (or create) the registry from an explicit path.
    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        let file: ProviderFile = if path.exists() {
            let raw = fs::read_to_string(path)?;
            serde_json::from_str(&raw)?
        } else {
            ProviderFile::default()
        };
        Ok(Self { path: path.to_owned(), file })
    }

    // ── Accessors ────────────────────────────────────────────────────────

    /// The currently active default provider id, if set.
    pub fn default_provider_id(&self) -> Option<&str> {
        self.file.default_provider.as_deref()
    }

    /// Get a provider by id.
    pub fn get(&self, id: &str) -> Option<&LlmProviderEntry> {
        self.file.providers.iter().find(|p| p.id == id)
    }

    /// Return all providers.
    pub fn all(&self) -> &[LlmProviderEntry] {
        &self.file.providers
    }

    // ── Mutations ────────────────────────────────────────────────────────

    /// Add or replace a provider. If `set_as_default` is true, also
    /// updates `default_provider` to this id.
    pub fn upsert(
        &mut self,
        entry: LlmProviderEntry,
        set_as_default: bool,
    ) -> anyhow::Result<()> {
        if set_as_default {
            self.file.default_provider = Some(entry.id.clone());
        }
        if let Some(existing) = self.file.providers.iter_mut().find(|p| p.id == entry.id) {
            *existing = entry;
        } else {
            self.file.providers.push(entry);
        }
        self.persist()
    }

    /// Remove a provider by id. If it was the default, clears `default_provider`.
    pub fn remove(&mut self, id: &str) -> anyhow::Result<bool> {
        let before = self.file.providers.len();
        self.file.providers.retain(|p| p.id != id);
        if self.file.default_provider.as_deref() == Some(id) {
            self.file.default_provider = None;
        }
        let removed = self.file.providers.len() < before;
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Set the default provider by id. Returns an error if the id is not
    /// present in the registry.
    pub fn set_default(&mut self, id: &str) -> anyhow::Result<()> {
        if self.file.providers.iter().any(|p| p.id == id) {
            self.file.default_provider = Some(id.to_owned());
            self.persist()
        } else {
            anyhow::bail!("provider '{}' not found in registry", id)
        }
    }

    // ── Keychain ─────────────────────────────────────────────────────────

    /// Retrieve the API token / OAuth access token for a provider from the
    /// macOS Keychain. Returns `None` if no item is stored yet.
    pub fn get_token(&self, provider_id: &str) -> anyhow::Result<Option<String>> {
        keychain_get(keychain_service(), &keychain_account(provider_id))
    }

    /// Store (or replace) the API token / OAuth access token for a provider
    /// in the macOS Keychain under the item label `cronymax-provider-<id>`.
    pub fn store_token(&self, provider_id: &str, token: &str) -> anyhow::Result<()> {
        keychain_set(keychain_service(), &keychain_account(provider_id), token)
    }

    /// Delete the keychain item for a provider, if any. Silently succeeds
    /// if the item did not exist.
    pub fn delete_token(&self, provider_id: &str) -> anyhow::Result<()> {
        keychain_delete(keychain_service(), &keychain_account(provider_id))
    }

    // ── Persistence ──────────────────────────────────────────────────────

    fn persist(&self) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(&self.file)?;
        atomic_write(&self.path, json.as_bytes())
    }
}

// ── Keychain helpers ─────────────────────────────────────────────────────────

fn keychain_service() -> &'static str {
    "cronymax"
}

fn keychain_account(provider_id: &str) -> String {
    format!("cronymax-provider-{provider_id}")
}

#[cfg(target_os = "macos")]
fn keychain_get(service: &str, account: &str) -> anyhow::Result<Option<String>> {
    use security_framework::passwords::get_generic_password;
    match get_generic_password(service, account) {
        Ok(bytes) => {
            let s = String::from_utf8(bytes)
                .map_err(|e| anyhow::anyhow!("keychain item is not valid UTF-8: {e}"))?;
            Ok(Some(s))
        }
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("keychain read failed: {e}")),
    }
}

#[cfg(target_os = "macos")]
fn keychain_set(service: &str, account: &str, token: &str) -> anyhow::Result<()> {
    use security_framework::passwords::set_generic_password;
    set_generic_password(service, account, token.as_bytes())
        .map_err(|e| anyhow::anyhow!("keychain write failed: {e}"))
}

#[cfg(target_os = "macos")]
fn keychain_delete(service: &str, account: &str) -> anyhow::Result<()> {
    use security_framework::passwords::delete_generic_password;
    match delete_generic_password(service, account) {
        Ok(()) => Ok(()),
        Err(e) if is_not_found(&e) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("keychain delete failed: {e}")),
    }
}

#[cfg(target_os = "macos")]
fn is_not_found(e: &security_framework::base::Error) -> bool {
    // errSecItemNotFound = -25300
    e.code() == -25300
}

// Non-macOS stubs (CI / Linux dev machines).
#[cfg(not(target_os = "macos"))]
fn keychain_get(_service: &str, _account: &str) -> anyhow::Result<Option<String>> {
    Ok(None)
}

#[cfg(not(target_os = "macos"))]
fn keychain_set(_service: &str, _account: &str, _token: &str) -> anyhow::Result<()> {
    anyhow::bail!("keychain not supported on this platform")
}

#[cfg(not(target_os = "macos"))]
fn keychain_delete(_service: &str, _account: &str) -> anyhow::Result<()> {
    Ok(())
}

// ── Atomic write ─────────────────────────────────────────────────────────────

/// Write `data` to `path` atomically using a POSIX rename.
fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().ok_or_else(|| anyhow::anyhow!("path has no parent"))?;
    // Construct a temp path in the same directory to allow atomic rename.
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

// ── Home directory ───────────────────────────────────────────────────────────

fn dirs_home() -> anyhow::Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| anyhow::anyhow!("HOME environment variable not set"))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry(dir: &Path) -> LlmProviderRegistry {
        LlmProviderRegistry::load_from(&dir.join("providers.json")).unwrap()
    }

    fn openai_entry() -> LlmProviderEntry {
        LlmProviderEntry {
            id: "openai".into(),
            kind: LlmProviderKind::OpenaiCompat,
            base_url: "https://api.openai.com/v1".into(),
            model_override: None,
        }
    }

    fn copilot_entry() -> LlmProviderEntry {
        LlmProviderEntry {
            id: "copilot".into(),
            kind: LlmProviderKind::GithubCopilot,
            base_url: "https://api.githubcopilot.com".into(),
            model_override: None,
        }
    }

    #[test]
    fn empty_registry_on_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let reg = make_registry(dir.path());
        assert!(reg.all().is_empty());
        assert!(reg.default_provider_id().is_none());
    }

    #[test]
    fn upsert_adds_provider() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut reg = make_registry(dir.path());
        reg.upsert(openai_entry(), false).unwrap();
        assert_eq!(reg.all().len(), 1);
        assert_eq!(reg.get("openai").unwrap().id, "openai");
    }

    #[test]
    fn upsert_replaces_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut reg = make_registry(dir.path());
        reg.upsert(openai_entry(), false).unwrap();
        let updated = LlmProviderEntry {
            id: "openai".into(),
            kind: LlmProviderKind::OpenaiCompat,
            base_url: "https://custom.openai.com/v1".into(),
            model_override: Some("gpt-4-turbo".into()),
        };
        reg.upsert(updated, false).unwrap();
        assert_eq!(reg.all().len(), 1);
        assert_eq!(reg.get("openai").unwrap().base_url, "https://custom.openai.com/v1");
    }

    #[test]
    fn upsert_with_default_flag_sets_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut reg = make_registry(dir.path());
        reg.upsert(openai_entry(), true).unwrap();
        assert_eq!(reg.default_provider_id(), Some("openai"));
    }

    #[test]
    fn remove_clears_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut reg = make_registry(dir.path());
        reg.upsert(openai_entry(), true).unwrap();
        reg.remove("openai").unwrap();
        assert!(reg.all().is_empty());
        assert!(reg.default_provider_id().is_none());
    }

    #[test]
    fn set_default_unknown_id_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut reg = make_registry(dir.path());
        assert!(reg.set_default("nonexistent").is_err());
    }

    #[test]
    fn registry_persists_and_reloads() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let mut reg = make_registry(dir.path());
            reg.upsert(openai_entry(), false).unwrap();
            reg.upsert(copilot_entry(), true).unwrap();
        }
        let reg2 = make_registry(dir.path());
        assert_eq!(reg2.all().len(), 2);
        assert_eq!(reg2.default_provider_id(), Some("copilot"));
        assert!(reg2.get("openai").is_some());
    }
}
