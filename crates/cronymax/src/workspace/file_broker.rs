//! Sandboxed async file I/O.
//!
//! Mirrors `app/workspace/file_broker.h`. The scope enforcement (path
//! must remain within `workspace_root`) is done here, not at the call
//! site. All I/O is async via `tokio::fs`.

use std::path::{Path, PathBuf};

use anyhow::bail;
use tokio::fs;

/// Async file read/write bounded to a workspace root.
#[derive(Clone, Debug)]
pub struct FileBroker {
    workspace_root: PathBuf,
}

impl FileBroker {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
        }
    }

    /// Read `path` as UTF-8 text. The path must be within `workspace_root`.
    pub async fn read_text(&self, path: &Path) -> anyhow::Result<String> {
        let canonical = self.enforce_scope(path)?;
        Ok(fs::read_to_string(&canonical).await?)
    }

    /// Write `content` to `path`, creating parent directories as needed.
    /// The path must be within `workspace_root`.
    pub async fn write_text(&self, path: &Path, content: &str) -> anyhow::Result<()> {
        let canonical = self.enforce_scope(path)?;
        if let Some(parent) = canonical.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&canonical, content).await?;
        Ok(())
    }

    // ── internals ─────────────────────────────────────────────────────────

    fn enforce_scope(&self, path: &Path) -> anyhow::Result<PathBuf> {
        // Normalise both paths without resolving symlinks (lexical only).
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };

        // Walk the components to check for `..` escapes without touching
        // the filesystem.
        let mut components: Vec<_> = Vec::new();
        for comp in abs.components() {
            use std::path::Component::*;
            match comp {
                ParentDir => {
                    if components.pop().is_none() {
                        bail!("path escapes workspace root: {}", path.display());
                    }
                }
                CurDir => {}
                other => components.push(other),
            }
        }
        let resolved: PathBuf = components.iter().collect();

        // The resolved path must be prefixed by workspace_root.
        if !resolved.starts_with(&self.workspace_root) {
            bail!("path is outside workspace root: {}", path.display());
        }
        Ok(resolved)
    }
}
