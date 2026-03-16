// Profile permissions — controls which skill categories are available per profile.

use serde::{Deserialize, Serialize};

/// Permissions derived from a [`Profile`](super::store::Profile).
///
/// Determines which skill categories the profile's agent loops and sessions
/// are allowed to invoke.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Allowed skill categories.
    /// Valid values: `sandbox`, `chat`, `browser`, `terminal`, `tab`,
    /// `webview`, `external`, `general`, `channels`, `scheduler`.
    pub allowed_skills: Vec<String>,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            allowed_skills: all_skill_categories(),
        }
    }
}

/// All available skill categories (default allowlist).
pub fn all_skill_categories() -> Vec<String> {
    vec![
        "sandbox".into(),
        "chat".into(),
        "browser".into(),
        "terminal".into(),
        "tab".into(),
        "webview".into(),
        "external".into(),
        "general".into(),
        "channels".into(),
        "scheduler".into(),
    ]
}

impl Permissions {
    /// Check whether a specific skill category is allowed.
    pub fn is_skill_allowed(&self, category: &str) -> bool {
        self.allowed_skills.iter().any(|s| s == category)
    }

    /// Check whether any of the given categories are allowed.
    pub fn any_allowed(&self, categories: &[&str]) -> bool {
        categories.iter().any(|c| self.is_skill_allowed(c))
    }

    /// Return a filtered list of skills allowed by this permission set.
    pub fn filter_skills<'a>(&self, skills: &'a [String]) -> Vec<&'a String> {
        skills
            .iter()
            .filter(|s| self.is_skill_allowed(s))
            .collect()
    }
}
