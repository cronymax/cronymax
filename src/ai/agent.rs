// Agent manifest and registry — installable agent packages with skills and system prompts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Manifest types ────────────────────────────────────────────────────────

/// Top-level agent manifest parsed from `agent.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Agent metadata block.
    pub agent: AgentMeta,
    /// System prompt template.
    #[serde(default)]
    pub system_prompt: Option<SystemPrompt>,
    /// Skills bundled in this agent.
    #[serde(default)]
    pub skills: Vec<AgentSkill>,
    /// Optional cron schedule.
    #[serde(default)]
    pub schedule: Option<AgentSchedule>,
}

/// Agent metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeta {
    /// Unique snake_case identifier.
    pub name: String,
    /// Semantic version (e.g., "0.2.1").
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Author name/email.
    #[serde(default)]
    pub author: String,
    /// License identifier.
    #[serde(default)]
    pub license: Option<String>,
    /// Project URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Whether the agent is active (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum cronymax version required.
    #[serde(default)]
    pub min_app_version: Option<String>,
}

/// A skill definition within an agent manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// Skill name (namespaced as `agent_name.skill_name` in registry).
    pub name: String,
    /// LLM-facing description.
    #[serde(default)]
    pub description: String,
    /// JSON Schema for parameters.
    #[serde(default = "default_empty_object")]
    pub parameters: Value,
}

/// System prompt template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemPrompt {
    /// Prompt template with `{{variable}}` placeholders.
    pub template: String,
}

/// Optional cron schedule for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSchedule {
    /// 5-field POSIX cron expression.
    pub cron: String,
    /// Human-readable schedule description.
    #[serde(default)]
    pub description: String,
    /// IANA timezone (future: chrono-tz).
    #[serde(default)]
    pub timezone: Option<String>,
}

// ── Registry types ────────────────────────────────────────────────────────

/// A single entry in `registry.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledAgent {
    /// Agent name (matches AgentMeta.name).
    pub name: String,
    /// Relative path to agent.toml from agents directory.
    pub path: String,
    /// Whether the agent is currently enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// ISO 8601 timestamp of installation.
    pub installed_at: String,
    /// ISO 8601 timestamp of last scheduled execution.
    #[serde(default)]
    pub last_run: Option<String>,
}

/// Persisted registry file (`registry.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryFile {
    #[serde(default)]
    pub installed: Vec<InstalledAgent>,
}

/// In-memory registry of installed agents.
pub struct AgentRegistry {
    /// Base directory for agent packages (`~/.config/cronymax/agents/`).
    pub agents_dir: PathBuf,
    /// List of installed agents from registry.toml.
    pub installed: Vec<InstalledAgent>,
    /// Loaded manifests keyed by agent name.
    pub manifests: HashMap<String, AgentManifest>,
}

impl AgentRegistry {
    /// Create a new registry pointing at the given agents directory.
    pub fn new(agents_dir: PathBuf) -> Self {
        Self {
            agents_dir,
            installed: Vec::new(),
            manifests: HashMap::new(),
        }
    }

    /// Create a registry using the default agents directory under the config dir.
    pub fn default_dir() -> Self {
        let agents_dir = crate::renderer::platform::config_dir().join("agents");
        Self::new(agents_dir)
    }

    /// Load the registry from `registry.toml` and parse all installed manifests.
    pub fn load(&mut self) -> anyhow::Result<()> {
        let registry_path = self.agents_dir.join("registry.toml");
        if !registry_path.exists() {
            self.installed.clear();
            self.manifests.clear();
            return Ok(());
        }
        let contents = std::fs::read_to_string(&registry_path)?;
        let file: RegistryFile = toml::from_str(&contents)?;
        self.installed = file.installed;
        // Load each manifest.
        self.manifests.clear();
        for entry in &self.installed {
            let manifest_path = self.agents_dir.join(&entry.path);
            if manifest_path.exists() {
                match AgentManifest::from_path(&manifest_path) {
                    Ok(manifest) => {
                        self.manifests.insert(entry.name.clone(), manifest);
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: failed to load manifest for agent '{}': {}",
                            entry.name, e
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Save the registry to `registry.toml`.
    fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.agents_dir)?;
        let file = RegistryFile {
            installed: self.installed.clone(),
        };
        let contents = toml::to_string_pretty(&file)?;
        std::fs::write(self.agents_dir.join("registry.toml"), contents)?;
        Ok(())
    }

    /// Install an agent from a local directory.
    ///
    /// Copies the directory to `agents_dir/{name}/` and adds a registry entry.
    pub fn install(&mut self, source_dir: &Path) -> anyhow::Result<String> {
        let manifest_path = source_dir.join("agent.toml");
        if !manifest_path.exists() {
            anyhow::bail!("No agent.toml found in selected directory");
        }
        let manifest = AgentManifest::from_path(&manifest_path)?;
        manifest.validate()?;
        let name = manifest.agent.name.clone();

        // Check for duplicates.
        if self.installed.iter().any(|a| a.name == name) {
            anyhow::bail!("Agent '{}' is already installed", name);
        }

        // Copy directory to agents_dir.
        let dest = self.agents_dir.join(&name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        copy_dir_recursive(source_dir, &dest)?;

        let entry = InstalledAgent {
            name: name.clone(),
            path: format!("{}/agent.toml", name),
            enabled: manifest.agent.enabled,
            installed_at: now_iso8601(),
            last_run: None,
        };
        self.installed.push(entry);
        self.manifests.insert(name.clone(), manifest);
        self.save()?;
        Ok(name)
    }

    /// Uninstall an agent by name.
    pub fn uninstall(&mut self, name: &str) -> anyhow::Result<()> {
        let idx = self
            .installed
            .iter()
            .position(|a| a.name == name)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' is not installed", name))?;
        self.installed.remove(idx);
        self.manifests.remove(name);

        // Remove directory.
        let agent_dir = self.agents_dir.join(name);
        if agent_dir.exists() {
            std::fs::remove_dir_all(&agent_dir)?;
        }
        self.save()?;
        Ok(())
    }

    /// Look up a manifest by agent name.
    pub fn lookup(&self, name: &str) -> Option<&AgentManifest> {
        self.manifests.get(name)
    }

    /// Enable an agent.
    pub fn enable(&mut self, name: &str) -> anyhow::Result<()> {
        self.set_enabled(name, true)
    }

    /// Disable an agent.
    pub fn disable(&mut self, name: &str) -> anyhow::Result<()> {
        self.set_enabled(name, false)
    }

    fn set_enabled(&mut self, name: &str, enabled: bool) -> anyhow::Result<()> {
        let entry = self
            .installed
            .iter_mut()
            .find(|a| a.name == name)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' is not installed", name))?;
        entry.enabled = enabled;
        if let Some(manifest) = self.manifests.get_mut(name) {
            manifest.agent.enabled = enabled;
        }
        self.save()?;
        Ok(())
    }

    /// List all installed agents.
    pub fn list(&self) -> &[InstalledAgent] {
        &self.installed
    }
}

// ── Manifest parsing & validation ─────────────────────────────────────────

impl AgentManifest {
    /// Parse an agent manifest from a TOML file.
    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read agent manifest: {}", e))?;
        let manifest: Self = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse agent manifest: {}", e))?;
        Ok(manifest)
    }

    /// Validate the manifest fields.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.agent.name.is_empty() {
            anyhow::bail!("Agent name must not be empty");
        }
        if self.agent.version.is_empty() {
            anyhow::bail!("Agent version must not be empty");
        }
        for skill in &self.skills {
            if skill.name.is_empty() {
                anyhow::bail!(
                    "Skill name must not be empty in agent '{}'",
                    self.agent.name
                );
            }
        }
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

fn default_empty_object() -> Value {
    serde_json::json!({})
}

fn now_iso8601() -> String {
    // Simple UTC timestamp without chrono dependency.
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Format as ISO 8601 (approximate — no chrono).
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let mins = (time_secs % 3600) / 60;
    let s = time_secs % 60;
    // Approximate date from days since epoch (1970-01-01).
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, mins, s
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
