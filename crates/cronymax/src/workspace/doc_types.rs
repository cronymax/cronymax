//! Doc-type schema registry.
//!
//! Mirrors `app/document/doc_type_registry.h` + `doc_type_schema.h`.
//! Scans `builtin_dir/*.yaml` then merges `workspace_dir/*.{yaml,md}`,
//! with workspace copies overriding built-ins of the same name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;

/// Parsed doc-type schema.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocTypeSchema {
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub user_defined: bool,
}

// ── YAML / Markdown parsing ──────────────────────────────────────────────────

/// Full YAML schema shape (used for `.yaml` files).
#[derive(Debug, Deserialize, Default)]
struct RawDocTypeYaml {
    name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
}

/// Parse a `.yaml` file.
fn parse_doc_type_yaml(text: &str) -> Option<DocTypeSchema> {
    let raw: RawDocTypeYaml = serde_yml::from_str(text).ok()?;
    if raw.name.is_empty() {
        return None;
    }
    let display_name = if raw.display_name.is_empty() {
        raw.name.clone()
    } else {
        raw.display_name
    };
    Some(DocTypeSchema {
        name: raw.name,
        display_name,
        description: raw.description,
        user_defined: false, // overwritten by caller
    })
}

/// Parse a `.md` file with YAML front matter:
/// ```
/// ---
/// name: my-type
/// display_name: My Type
/// ---
/// Markdown body becomes description.
/// ```
fn parse_doc_type_markdown(text: &str) -> Option<DocTypeSchema> {
    let inner = text.trim_start();
    if !inner.starts_with("---") {
        return None;
    }
    let rest = &inner[3..];
    let end = rest.find("\n---")?;
    let fm = &rest[..end];
    let body = rest[end + 4..].trim().to_owned();

    #[derive(Deserialize, Default)]
    struct Fm {
        name: String,
        #[serde(default)]
        display_name: String,
    }
    let fm: Fm = serde_yml::from_str(fm).ok()?;
    if fm.name.is_empty() {
        return None;
    }
    let display_name = if fm.display_name.is_empty() {
        fm.name.clone()
    } else {
        fm.display_name
    };
    Some(DocTypeSchema {
        name: fm.name,
        display_name,
        description: body,
        user_defined: false,
    })
}

// ── Registry ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DocTypeRegistry {
    builtin_dir: PathBuf,
    workspace_dir: PathBuf,
    schemas: HashMap<String, DocTypeSchema>,
}

impl DocTypeRegistry {
    pub fn new(builtin_dir: impl Into<PathBuf>, workspace_dir: impl Into<PathBuf>) -> Self {
        Self {
            builtin_dir: builtin_dir.into(),
            workspace_dir: workspace_dir.into(),
            schemas: HashMap::new(),
        }
    }

    /// Reload built-ins then merge workspace overrides.
    pub async fn refresh(&mut self) -> usize {
        let mut next: HashMap<String, DocTypeSchema> = HashMap::new();
        scan_dir(&self.builtin_dir, false, &mut next).await;
        scan_dir(&self.workspace_dir, true, &mut next).await;
        let count = next.len();
        self.schemas = next;
        count
    }

    pub fn get(&self, name: &str) -> Option<&DocTypeSchema> {
        self.schemas.get(name)
    }

    /// Sorted names.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.schemas.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn is_user_defined(&self, name: &str) -> bool {
        self.schemas
            .get(name)
            .map(|s| s.user_defined)
            .unwrap_or(false)
    }

    /// Write a user-defined doc-type as a `.md` file and refresh.
    pub async fn save(
        &mut self,
        name: &str,
        display_name: &str,
        description: &str,
    ) -> anyhow::Result<()> {
        validate_safe_name(name)?;
        if let Some(parent) = self.workspace_dir.parent() {
            fs::create_dir_all(parent).await.ok();
        }
        fs::create_dir_all(&self.workspace_dir).await?;
        let path = self.workspace_dir.join(format!("{name}.md"));
        let content = format!(
            "---\nname: {}\ndisplay_name: {}\n---\n\n{}\n",
            serde_json::to_string(name).unwrap_or_default(),
            serde_json::to_string(display_name).unwrap_or_default(),
            description
        );
        let tmp = path.with_extension("md.tmp");
        fs::write(&tmp, &content).await?;
        fs::rename(&tmp, &path).await?;
        self.refresh().await;
        Ok(())
    }

    /// Delete a user-defined doc-type and refresh.
    pub async fn delete(&mut self, name: &str) -> anyhow::Result<()> {
        validate_safe_name(name)?;
        for ext in &["md", "yaml"] {
            let p = self.workspace_dir.join(format!("{name}.{ext}"));
            match fs::remove_file(&p).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.schemas.remove(name);
        Ok(())
    }

    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }
    pub fn builtin_dir(&self) -> &Path {
        &self.builtin_dir
    }
}

async fn scan_dir(dir: &Path, user: bool, out: &mut HashMap<String, DocTypeSchema>) {
    let Ok(mut rd) = fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        let fname = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let schema_opt = if fname.ends_with(".yaml") {
            fs::read_to_string(&path)
                .await
                .ok()
                .and_then(|t| parse_doc_type_yaml(&t))
        } else if fname.ends_with(".md") {
            fs::read_to_string(&path)
                .await
                .ok()
                .and_then(|t| parse_doc_type_markdown(&t))
        } else {
            None
        };
        if let Some(mut schema) = schema_opt {
            schema.user_defined = user;
            out.insert(schema.name.clone(), schema);
        }
    }
}

fn validate_safe_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name.len() > 64 {
        anyhow::bail!("invalid doc-type name length");
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("doc-type name contains invalid characters");
    }
    if name.starts_with('-') {
        anyhow::bail!("doc-type name must not start with '-'");
    }
    Ok(())
}
