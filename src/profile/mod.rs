// Profile — disk persistence, session/memory CRUD.

mod manager;
pub mod permissions;
pub mod sandbox;

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ai::context::ChatMessage;
use crate::profile::permissions::Permissions;
pub use manager::ProfileManager;

/// Filename constants for profile data files.
const PROFILE_TOML: &str = "profile.toml";
const POLICY_TOML: &str = "policy.toml";
const SESSION_JSON: &str = "session.json";
const MEMORY_JSON: &str = "memory.json";
const WEBDATA_DIR: &str = "webdata";

/// A named profile with sandbox, permissions, memory config, and creation timestamp.
///
/// The Profile is the top-level data isolation boundary. Each window operates
/// under exactly one Profile, which owns:
/// - [`sandbox`] — FS + network rules for child process spawning
/// - [`permissions`] — which skill categories are available
/// - [`memory`] — memory service configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// Legacy path reference (kept for backward compat, ignored on new saves).
    pub sandbox_policy: Option<String>,
    /// Embedded sandbox policy — saved directly in profile.toml.
    #[serde(default)]
    pub sandbox: Option<crate::profile::sandbox::policy::SandboxPolicy>,
    pub created_at: String,
    /// Memory service configuration for this profile.
    pub memory: MemoryConfig,
    /// Permissions controlling available skill categories.
    #[serde(default)]
    pub permissions: Permissions,
    /// Legacy: skill categories allowlist. Migrated to `permissions` on load.
    #[serde(default, skip_serializing)]
    pub allowed_skills: Vec<String>,
}

impl Profile {
    /// Migrate legacy `allowed_skills` field into `permissions`.
    /// Call after deserialization to normalize the config.
    pub fn migrate_legacy(&mut self) {
        if !self.allowed_skills.is_empty() && self.permissions.allowed_skills.is_empty() {
            self.permissions.allowed_skills = std::mem::take(&mut self.allowed_skills);
        }
    }

    /// Returns the default set of allowed skill categories for new profiles.
    pub fn default_allowed_skills() -> Vec<String> {
        Permissions::default().allowed_skills
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "Default".into(),
            sandbox_policy: None,
            sandbox: None,
            created_at: String::new(),
            memory: MemoryConfig::default(),
            permissions: Permissions::default(),
            allowed_skills: Vec::new(),
        }
    }
}

// Re-export memory types from the services layer for backward compatibility.
// The canonical definitions now live in `crate::services::memory`.
pub use crate::services::memory::{MemoryConfig, MemoryEntry, MemoryStore, MemoryTag};

/// An LLM session that persists across app restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSession {
    pub profile_id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub token_count: u32,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}
