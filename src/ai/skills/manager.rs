//! Skills lifecycle manager — install, uninstall, enable/disable, update, and registry persistence.
//!
//! `SkillsManager` orchestrates the complete external skills lifecycle:
//! - Loading skills from the filesystem and registering them in `SkillRegistry`
//! - Installing skills from ClawHub (download + parse + register)
//! - Uninstalling skills (remove directory + registry entry)
//! - Enabling/disabling skills
//! - Checking for and applying updates from ClawHub
//! - Persisting skill state in `skills_registry.toml`
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ai::clawhub::ClawHubClient;
use crate::ai::skills;
use crate::ai::skills::SkillRegistry;
use crate::ai::skills::loader::{ExternalSkill, SkillLoader, SkillSource};
use crate::config::AppConfig;

// ── Registry Types ───────────────────────────────────────────────────────────

/// Entry in `skills_registry.toml` tracking an installed skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillEntry {
    pub version: String,
    pub source: String,
    #[serde(default)]
    pub slug: Option<String>,
    pub installed_at: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub updated_at: Option<String>,
}

fn default_true() -> bool {
    true
}

/// The `skills_registry.toml` file structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsRegistryFile {
    #[serde(default)]
    pub skills: HashMap<String, InstalledSkillEntry>,
}

// ── Skills Manager ───────────────────────────────────────────────────────────

/// Orchestrator for the complete external skills lifecycle.
pub struct SkillsManager {
    loader: SkillLoader,
    clawhub: ClawHubClient,
    skills_dir: PathBuf,
    registry_path: PathBuf,
}

impl SkillsManager {
    /// Create a new SkillsManager.
    ///
    /// - `skills_dir`: directory containing installed skills (e.g., `~/.config/cronymax/skills/`)
    /// - `api_base`: ClawHub API base URL (default: `"https://clawhub.ai"`)
    pub fn new(skills_dir: PathBuf, api_base: String) -> Self {
        let registry_path = skills_dir.join("skills_registry.toml");
        let loader = SkillLoader::new(skills_dir.clone());
        let clawhub = ClawHubClient::new(&api_base);

        Self {
            loader,
            clawhub,
            skills_dir,
            registry_path,
        }
    }

    /// Get the skills directory path.
    pub fn skills_dir(&self) -> &std::path::Path {
        &self.skills_dir
    }

    /// Get the ClawHub API base URL.
    pub fn api_base(&self) -> &str {
        self.clawhub.api_base()
    }

    // ── Registry Persistence ─────────────────────────────────────────────

    /// Load the skills registry from `skills_registry.toml`.
    pub fn load_registry(&self) -> anyhow::Result<SkillsRegistryFile> {
        if !self.registry_path.exists() {
            return Ok(SkillsRegistryFile::default());
        }

        let contents = std::fs::read_to_string(&self.registry_path)?;
        let registry: SkillsRegistryFile = toml::from_str(&contents)?;
        Ok(registry)
    }

    /// Save the skills registry to `skills_registry.toml`.
    pub fn save_registry(&self, registry: &SkillsRegistryFile) -> anyhow::Result<()> {
        if let Some(parent) = self.registry_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(registry)?;
        std::fs::write(&self.registry_path, contents)?;
        Ok(())
    }

    // ── Install / Uninstall ──────────────────────────────────────────────

    /// Install a skill from ClawHub.
    ///
    /// Downloads the skill files, parses SKILL.md, and adds a registry entry.
    pub async fn install(&self, slug: &str) -> anyhow::Result<ExternalSkill> {
        // Download skill from ClawHub.
        let skill_dir = self.clawhub.download(slug, &self.skills_dir).await?;
        let skill_md_path = skill_dir.join("SKILL.md");

        // Parse the downloaded SKILL.md.
        let mut skill = skills::loader::parse_skill_md(&skill_md_path)
            .map_err(|e| anyhow::anyhow!("Failed to parse downloaded skill: {}", e))?;

        // Set source to ClawHub.
        // Get version from ClawHub detail.
        let detail = self.clawhub.get_skill(slug).await?;
        skill.source = SkillSource::ClawHub {
            slug: slug.to_string(),
            version: detail.version.clone(),
        };

        // Add registry entry.
        let mut registry = self.load_registry()?;
        let entry = InstalledSkillEntry {
            version: detail.version,
            source: "clawhub".to_string(),
            slug: Some(slug.to_string()),
            installed_at: chrono::Utc::now().to_rfc3339(),
            enabled: true,
            updated_at: None,
        };
        registry
            .skills
            .insert(skill.frontmatter.name.clone(), entry);
        self.save_registry(&registry)?;

        log::info!("Installed skill '{}' from ClawHub", slug);
        Ok(skill)
    }

    /// Uninstall a skill — removes its directory and registry entry.
    pub fn uninstall(&self, name: &str) -> anyhow::Result<()> {
        // Remove directory.
        let skill_dir = self.skills_dir.join(name);
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir)?;
        }

        // Remove registry entry.
        let mut registry = self.load_registry()?;
        registry.skills.remove(name);
        self.save_registry(&registry)?;

        log::info!("Uninstalled skill '{}'", name);
        Ok(())
    }

    // ── Enable / Disable ─────────────────────────────────────────────────

    /// Set the enabled state of a skill.
    pub fn set_enabled(&self, name: &str, enabled: bool) -> anyhow::Result<()> {
        let mut registry = self.load_registry()?;
        if let Some(entry) = registry.skills.get_mut(name) {
            entry.enabled = enabled;
            self.save_registry(&registry)?;
            log::info!(
                "Skill '{}' {}",
                name,
                if enabled { "enabled" } else { "disabled" }
            );
        } else {
            anyhow::bail!("Skill '{}' not found in registry", name);
        }
        Ok(())
    }

    /// List all installed skills with their registry entries.
    pub fn list(&self) -> anyhow::Result<Vec<(String, InstalledSkillEntry)>> {
        let registry = self.load_registry()?;
        Ok(registry.skills.into_iter().collect())
    }

    // ── Update ───────────────────────────────────────────────────────────

    /// Update a single ClawHub-sourced skill if a newer version exists.
    ///
    /// Returns `true` if the skill was updated, `false` if already up-to-date.
    pub async fn update(&self, name: &str) -> anyhow::Result<bool> {
        let registry = self.load_registry()?;
        let entry = registry
            .skills
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found in registry", name))?;

        let slug = entry
            .slug
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' is not from ClawHub", name))?;

        match self.clawhub.check_update(slug, &entry.version).await? {
            Some(new_version) => {
                log::info!(
                    "Updating skill '{}': {} → {}",
                    name,
                    entry.version,
                    new_version
                );
                // Re-download and update registry.
                self.clawhub.download(slug, &self.skills_dir).await?;

                let mut registry = self.load_registry()?;
                if let Some(entry) = registry.skills.get_mut(name) {
                    entry.version = new_version;
                    entry.updated_at = Some(chrono::Utc::now().to_rfc3339());
                }
                self.save_registry(&registry)?;
                Ok(true)
            }
            None => {
                log::info!("Skill '{}' is already up-to-date", name);
                Ok(false)
            }
        }
    }

    /// Update all ClawHub-sourced skills. Returns names of skills that were updated.
    pub async fn update_all(&self) -> anyhow::Result<Vec<String>> {
        let registry = self.load_registry()?;
        let mut updated = Vec::new();

        for (name, entry) in &registry.skills {
            if entry.source != "clawhub" {
                continue;
            }
            match self.update(name).await {
                Ok(true) => updated.push(name.clone()),
                Ok(false) => {}
                Err(e) => {
                    log::warn!("Failed to update skill '{}': {}", name, e);
                }
            }
        }

        Ok(updated)
    }

    // ── Load & Register ──────────────────────────────────────────────────

    /// Load all eligible skills from the filesystem and register them in SkillRegistry.
    ///
    /// Returns the number of skills registered.
    pub fn load_and_register(
        &self,
        registry: &mut SkillRegistry,
        config: &AppConfig,
    ) -> anyhow::Result<usize> {
        let skills = self
            .loader
            .load_all(config)
            .map_err(|e| anyhow::anyhow!("Failed to load skills: {}", e))?;

        // Filter out disabled skills per the registry.
        let skill_registry = self.load_registry()?;
        let enabled_skills: Vec<ExternalSkill> = skills
            .into_iter()
            .filter(|s| {
                skill_registry
                    .skills
                    .get(&s.frontmatter.name)
                    .map(|e| e.enabled)
                    .unwrap_or(true) // Not in registry = enabled by default (local skills)
            })
            .collect();

        let count = enabled_skills.len();
        skills::loader::register_external_skills(registry, &enabled_skills);

        log::info!("Registered {} external skills", count);
        Ok(count)
    }

    /// Search ClawHub for skills (delegates to ClawHubClient).
    pub async fn search(
        &self,
        query: &str,
    ) -> anyhow::Result<Vec<crate::ai::clawhub::ClawHubSkillResult>> {
        self.clawhub.search(query, 20).await
    }

    /// Hot-reload skills from the filesystem without app restart.
    ///
    /// Removes all previously registered "external" category skills, then
    /// re-loads and re-registers from disk.
    pub fn reload_skills(
        &self,
        registry: &mut SkillRegistry,
        config: &AppConfig,
    ) -> anyhow::Result<usize> {
        // Remove existing external skills from registry.
        registry.remove_by_category("external");
        // Re-load and register.
        self.load_and_register(registry, config)
    }
}
