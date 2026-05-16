//! Workspace prompt files (`*.prompt.md`).
//!
//! Prompt files live in `$workspaceDir/.cronymax/prompts/<slug>.prompt.md`.
//! They may contain an optional YAML frontmatter block delimited by `---`
//! at the top of the file, followed by a Markdown body.
//!
//! Example:
//! ```markdown
//! ---
//! name: Code Review
//! description: Reviews a diff for correctness and style
//! tags:
//!   - review
//!   - code
//! ---
//! You are a meticulous code reviewer...
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Parsed representation of a `*.prompt.md` file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PromptFile {
    /// Human-readable name (from frontmatter `name:` key).
    #[serde(default)]
    pub name: Option<String>,
    /// Short description (from frontmatter `description:` key).
    #[serde(default)]
    pub description: Option<String>,
    /// Optional tag list (from frontmatter `tags:` key).
    #[serde(default)]
    pub tags: Vec<String>,
    /// The Markdown body text (everything after the closing `---` delimiter).
    pub body: String,
}

/// Frontmatter-only struct for serde_yml deserialization.
#[derive(Clone, Debug, Default, Deserialize)]
struct Frontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

impl PromptFile {
    /// Load and parse a `*.prompt.md` file.
    ///
    /// Returns `None` if the file cannot be read; returns `Ok` with an empty
    /// frontmatter and the full file content as the body if parsing fails or
    /// there is no frontmatter.
    pub fn from_path(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        Some(Self::parse(&content))
    }

    /// Parse raw file content into a `PromptFile`.
    pub fn parse(content: &str) -> Self {
        // Check for frontmatter: must start with `---\n` (or `---\r\n`).
        let trimmed = content.trim_start_matches('\u{FEFF}'); // strip BOM if present
        if !trimmed.starts_with("---") {
            return Self {
                body: content.to_owned(),
                ..Default::default()
            };
        }
        // The opening `---` must be followed by a newline (bare `---` at start).
        let rest = &trimmed[3..];
        let rest = match rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
        {
            Some(r) => r,
            None => {
                return Self {
                    body: content.to_owned(),
                    ..Default::default()
                };
            }
        };
        // Find the closing `---` on its own line.
        let close = rest
            .find("\n---\n")
            .or_else(|| rest.find("\n---\r\n"))
            .or_else(|| {
                // Handle `---` at the very start of `rest` (empty yaml block).
                if rest.starts_with("---\n") || rest.starts_with("---\r\n") {
                    Some(usize::MAX) // sentinel: yaml is empty
                } else {
                    None
                }
            });
        let (yaml_src, body) = match close {
            None => {
                return Self {
                    body: content.to_owned(),
                    ..Default::default()
                };
            }
            Some(usize::MAX) => {
                // Empty yaml block: `---\n---\n`
                let after = rest
                    .strip_prefix("---\n")
                    .or_else(|| rest.strip_prefix("---\r\n"))
                    .unwrap_or("");
                ("", after.to_owned())
            }
            Some(pos) => {
                let yaml = &rest[..pos]; // excludes the leading `\n`
                let after_close = &rest[pos + 1..]; // skip `\n`
                let after_dashes = after_close
                    .strip_prefix("---\n")
                    .or_else(|| after_close.strip_prefix("---\r\n"))
                    .or_else(|| after_close.strip_prefix("---"))
                    .unwrap_or(after_close);
                (yaml, after_dashes.to_owned())
            }
        };

        match serde_yml::from_str::<Frontmatter>(yaml_src) {
            Ok(fm) => Self {
                name: fm.name,
                description: fm.description,
                tags: fm.tags,
                body,
            },
            Err(_) => Self {
                body: content.to_owned(),
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_with_frontmatter() {
        let src = "---\nname: Code Review\ndescription: Reviews diffs\ntags:\n  - review\n  - code\n---\nYou are a reviewer.\n";
        let p = PromptFile::parse(src);
        assert_eq!(p.name.as_deref(), Some("Code Review"));
        assert_eq!(p.description.as_deref(), Some("Reviews diffs"));
        assert_eq!(p.tags, vec!["review", "code"]);
        assert_eq!(p.body.trim(), "You are a reviewer.");
    }

    #[test]
    fn parse_without_frontmatter() {
        let src = "You are a helpful assistant.\n";
        let p = PromptFile::parse(src);
        assert!(p.name.is_none());
        assert!(p.tags.is_empty());
        assert_eq!(p.body, src);
    }

    #[test]
    fn parse_empty_frontmatter() {
        let src = "---\n---\nBody only.\n";
        let p = PromptFile::parse(src);
        assert!(p.name.is_none());
        assert_eq!(p.body.trim(), "Body only.");
    }

    #[test]
    fn from_path_roundtrip() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "---\nname: My Prompt\n---\nHello world.\n").unwrap();
        let p = PromptFile::from_path(f.path()).unwrap();
        assert_eq!(p.name.as_deref(), Some("My Prompt"));
        assert_eq!(p.body.trim(), "Hello world.");
    }

    #[test]
    fn from_path_missing_returns_none() {
        let r = PromptFile::from_path(std::path::Path::new("/nonexistent/path.prompt.md"));
        assert!(r.is_none());
    }
}
