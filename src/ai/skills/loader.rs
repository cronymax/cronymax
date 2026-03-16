//! SKILL.md parser, filesystem scanner, and gating logic for OpenClaw-compatible skills.
//!
//! Skills are directories containing a `SKILL.md` file with YAML frontmatter
//! and a Markdown body (LLM instructions). This module handles:
//!
//! - Parsing YAML frontmatter via `serde_yaml`
//! - Validating skill name (kebab-case) and description
//! - Gating checks (platform, env vars, binaries, config keys)
//! - Scanning `~/.config/cronymax/skills/` and loading eligible skills
//! - Converting `ExternalSkill` → `Skill` for registration in `SkillRegistry`
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::AppConfig;

// ── Frontmatter Types ────────────────────────────────────────────────────────

/// Parsed YAML frontmatter from a `SKILL.md` file.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default = "default_true", rename = "user-invocable")]
    pub user_invocable: bool,
    #[serde(default, rename = "disable-model-invocation")]
    pub disable_model_invocation: bool,
    #[serde(default)]
    pub metadata: Option<SkillMetadataWrapper>,
}

fn default_true() -> bool {
    true
}

/// Wrapper for the metadata field — OpenClaw supports multiple aliases.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillMetadataWrapper {
    #[serde(default)]
    pub openclaw: Option<SkillMetadata>,
    #[serde(default)]
    pub clawdbot: Option<SkillMetadata>,
    #[serde(default)]
    pub clawdis: Option<SkillMetadata>,
}

impl SkillMetadataWrapper {
    /// Get the effective metadata, preferring openclaw > clawdbot > clawdis.
    pub fn effective(&self) -> Option<&SkillMetadata> {
        self.openclaw
            .as_ref()
            .or(self.clawdbot.as_ref())
            .or(self.clawdis.as_ref())
    }
}

/// Metadata block from `metadata.openclaw` in SKILL.md frontmatter.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillMetadata {
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
    #[serde(default)]
    pub always: bool,
    #[serde(default, rename = "primaryEnv")]
    pub primary_env: Option<String>,
    #[serde(default, rename = "skillKey")]
    pub skill_key: Option<String>,
    #[serde(default)]
    pub requires: Option<SkillRequirements>,
    #[serde(default)]
    pub install: Option<Vec<InstallSpec>>,
}

/// Load-time gating requirements from `metadata.openclaw.requires`.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillRequirements {
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default, rename = "anyBins")]
    pub any_bins: Vec<String>,
    #[serde(default)]
    pub config: Vec<String>,
}

/// Installer specification for skill dependencies.
#[derive(Debug, Clone, Deserialize)]
pub struct InstallSpec {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub formula: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
    #[serde(default)]
    pub url: Option<String>,
}

// ── External Skill & Source ──────────────────────────────────────────────────

/// A loaded OpenClaw-compatible skill from the filesystem.
#[derive(Debug, Clone)]
pub struct ExternalSkill {
    pub frontmatter: SkillFrontmatter,
    pub instructions: String,
    pub source_path: PathBuf,
    pub source: SkillSource,
}

/// Origin of an installed skill.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub enum SkillSource {
    /// Installed from ClawHub registry.
    ClawHub { slug: String, version: String },
    /// Manually placed in skills directory.
    Local,
}

// ── Parse Errors ─────────────────────────────────────────────────────────────

/// Errors that can occur when parsing a SKILL.md file.
#[derive(Debug, thiserror::Error)]
pub enum SkillParseError {
    #[error("SKILL.md missing YAML frontmatter delimiters")]
    MissingDelimiters,

    #[error("Invalid YAML in SKILL.md frontmatter: {0}")]
    InvalidFrontmatter(#[from] serde_yaml::Error),

    #[error("Skill name is empty or not valid kebab-case: {0}")]
    InvalidSkillName(String),

    #[error("Skill description is empty or exceeds 500 characters")]
    InvalidSkillDescription,

    #[error("Failed to read SKILL.md: {0}")]
    IoError(#[from] std::io::Error),
}

// ── Parsing ──────────────────────────────────────────────────────────────────

/// Parse YAML frontmatter from SKILL.md content.
///
/// Returns `(frontmatter, markdown_body)` where `markdown_body` is everything
/// after the closing `---` delimiter.
pub fn parse_frontmatter(content: &str) -> Result<(SkillFrontmatter, String), SkillParseError> {
    let trimmed = content.trim_start();

    // Must start with ---
    if !trimmed.starts_with("---") {
        return Err(SkillParseError::MissingDelimiters);
    }

    // Find end of first --- line
    let after_first = &trimmed[3..];
    let after_first = after_first
        .strip_prefix('\n')
        .unwrap_or(after_first.strip_prefix("\r\n").unwrap_or(after_first));

    // Find second ---
    let end_pos = after_first
        .find("\n---")
        .ok_or(SkillParseError::MissingDelimiters)?;

    let yaml_str = &after_first[..end_pos];
    let body_start = end_pos + 4; // skip \n---
    let rest = &after_first[body_start..];
    // Skip the newline after closing ---
    let body = rest
        .strip_prefix('\n')
        .unwrap_or(rest.strip_prefix("\r\n").unwrap_or(rest));

    let frontmatter: SkillFrontmatter = serde_yaml::from_str(yaml_str)?;
    Ok((frontmatter, body.to_string()))
}

/// Validate a parsed frontmatter: name must be non-empty kebab-case, description non-empty.
pub fn validate_frontmatter(fm: &SkillFrontmatter) -> Result<(), SkillParseError> {
    // Kebab-case: lowercase letters, digits, hyphens, no leading/trailing hyphen
    let is_kebab_case = !fm.name.is_empty()
        && fm
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !fm.name.starts_with('-')
        && !fm.name.ends_with('-');

    if !is_kebab_case {
        return Err(SkillParseError::InvalidSkillName(fm.name.clone()));
    }

    if fm.description.is_empty() || fm.description.len() > 500 {
        return Err(SkillParseError::InvalidSkillDescription);
    }

    Ok(())
}

// ── Gating ───────────────────────────────────────────────────────────────────

/// Return the OpenClaw `os` string for the current platform.
fn current_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "unknown"
    }
}

/// Check if a skill meets its gating requirements.
///
/// A skill is eligible if:
/// - `always` is true (bypasses all checks), OR
/// - Platform matches (or no `os` filter), AND
/// - All `requires.env` vars exist, AND
/// - All `requires.bins` are on PATH, AND
/// - At least one of `requires.any_bins` is on PATH (if non-empty), AND
/// - All `requires.config` keys are truthy in AppConfig.
pub fn is_eligible(meta: &SkillMetadata, config: &AppConfig) -> bool {
    if meta.always {
        return true;
    }

    // Platform check.
    if let Some(ref os_list) = meta.os
        && !os_list.iter().any(|o| o == current_os())
    {
        return false;
    }

    if let Some(ref req) = meta.requires {
        // All env vars must exist.
        for var in &req.env {
            if std::env::var(var).is_err() {
                return false;
            }
        }
        // All bins must be on PATH.
        for bin in &req.bins {
            if which::which(bin).is_err() {
                return false;
            }
        }
        // At least one of any_bins must be on PATH.
        if !req.any_bins.is_empty() && !req.any_bins.iter().any(|b| which::which(b).is_ok()) {
            return false;
        }
        // All config keys must be truthy.
        for key in &req.config {
            if !config.is_truthy(key) {
                return false;
            }
        }
    }

    true
}

// ── Full SKILL.md Parsing ────────────────────────────────────────────────────

/// Parse a SKILL.md file into an `ExternalSkill`.
pub fn parse_skill_md(path: &Path) -> Result<ExternalSkill, SkillParseError> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter, instructions) = parse_frontmatter(&content)?;
    validate_frontmatter(&frontmatter)?;

    let source_path = path.parent().unwrap_or(path).to_path_buf();

    Ok(ExternalSkill {
        frontmatter,
        instructions,
        source_path,
        source: SkillSource::Local,
    })
}

// ── Skill Loader ─────────────────────────────────────────────────────────────

/// Filesystem scanner that loads all eligible external skills from the skills directory.
pub struct SkillLoader {
    skills_dir: PathBuf,
}

impl SkillLoader {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    /// Scan the skills directory and load all eligible, enabled skills.
    ///
    /// For each subdirectory containing a `SKILL.md`:
    /// 1. Parse frontmatter + body
    /// 2. Check eligibility (platform, bins, env, config gating)
    /// 3. Return eligible skills
    pub fn load_all(&self, config: &AppConfig) -> Result<Vec<ExternalSkill>, SkillParseError> {
        let mut skills = Vec::new();

        if !self.skills_dir.exists() {
            log::info!(
                "Skills directory does not exist: {}",
                self.skills_dir.display()
            );
            return Ok(skills);
        }

        let entries = std::fs::read_dir(&self.skills_dir)?;

        for entry in entries {
            let entry = entry?;
            let skill_md = entry.path().join("SKILL.md");

            if !skill_md.is_file() {
                continue;
            }

            match parse_skill_md(&skill_md) {
                Ok(skill) => {
                    // Check gating.
                    let eligible = skill
                        .frontmatter
                        .metadata
                        .as_ref()
                        .and_then(|m| m.effective())
                        .map(|meta| is_eligible(meta, config))
                        .unwrap_or(true); // No metadata = no gating requirements = eligible

                    if eligible {
                        log::info!("Loaded skill: {}", skill.frontmatter.name);
                        skills.push(skill);
                    } else {
                        log::info!(
                            "Skill '{}' not eligible on this platform/environment",
                            skill.frontmatter.name
                        );
                    }
                }
                Err(e) => {
                    log::warn!("Failed to parse skill at {}: {}", skill_md.display(), e);
                }
            }
        }

        Ok(skills)
    }
}

// ── SkillRegistry Integration ────────────────────────────────────────────────

/// Convert an `ExternalSkill` into a `Skill` struct for `SkillRegistry`.
impl From<&ExternalSkill> for crate::ai::skills::Skill {
    fn from(ext: &ExternalSkill) -> Self {
        crate::ai::skills::Skill {
            name: ext.frontmatter.name.clone(),
            description: ext.frontmatter.description.clone(),
            parameters_schema: serde_json::Value::Object(Default::default()),
            category: "external".to_string(),
        }
    }
}

/// Register a slice of external skills into the SkillRegistry.
///
/// External skills are registered with a no-op handler since they provide
/// instructions via system prompt injection rather than being callable tools.
pub fn register_external_skills(
    registry: &mut crate::ai::skills::SkillRegistry,
    skills: &[ExternalSkill],
) {
    use std::sync::Arc;

    for ext in skills {
        let skill: crate::ai::skills::Skill = ext.into();
        let name = skill.name.clone();

        // Check for name collisions with built-in skills.
        if registry.get(&name).is_some() {
            log::warn!(
                "Skill name '{}' collides with existing skill — skipping external skill",
                name
            );
            continue;
        }

        // External skills use a no-op handler because they provide context
        // via system prompt injection, not as callable tools.
        let handler: crate::ai::skills::SkillHandler = Arc::new(|_args| {
            Box::pin(async {
                Ok(serde_json::json!({
                    "error": "External skills are not directly callable"
                }))
            })
        });

        registry.register(skill, handler);
    }
}
