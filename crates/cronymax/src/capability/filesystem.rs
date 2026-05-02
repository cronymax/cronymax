//! Filesystem mediation, workspace scope enforcement, and secret
//! access (task 6.3).
//!
//! The agent loop may call `read_file`, `write_file`, and
//! `read_secret`. All paths are validated against the active
//! [`WorkspaceScope`] before the implementation performs any I/O.
//! Paths that escape the scope boundary are rejected with a structured
//! error.
//!
//! [`LocalFilesystem`] provides a self-contained `tokio::fs`-backed
//! implementation — no C++ host call required.
//!
//! ## Workspace scope
//!
//! Every file request is relative to the Space's workspace root
//! (surfaced as `WorkspaceScope::root`). The dispatcher resolves
//! absolute paths and ensures no `..` traversal exits the root before
//! calling any `FilesystemCapability` method.
//!
//! ## Secrets
//!
//! Secrets (API keys, tokens, etc.) are read-only from the runtime's
//! perspective. The default implementation reads named environment
//! variables; replace with a custom impl for keychain integration.

use std::io::SeekFrom;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

// ── Workspace scope ──────────────────────────────────────────────

/// A workspace root plus optional allow-list of sub-paths.
#[derive(Clone, Debug)]
pub struct WorkspaceScope {
    /// Absolute path to the workspace root directory.
    pub root: PathBuf,
}

impl WorkspaceScope {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve `rel` relative to `root` and verify it stays inside.
    /// Returns the absolute path on success.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, ScopeError> {
        let joined = self.root.join(rel);
        // Normalize without following symlinks.
        let resolved = normalize_path(&joined);
        if !resolved.starts_with(&self.root) {
            return Err(ScopeError::OutsideWorkspace {
                path: rel.to_owned(),
                root: self.root.display().to_string(),
            });
        }
        Ok(resolved)
    }
}

/// Normalize a path by resolving `.` and `..` without touching the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                components.pop();
            }
            Component::CurDir => {}
            other => components.push(other),
        }
    }
    components
}

#[derive(Debug, Error)]
pub enum ScopeError {
    #[error("path '{path}' escapes workspace root '{root}'")]
    OutsideWorkspace { path: String, root: String },
}

// ── Read / write file requests ────────────────────────────────────────────

/// Read a file inside the workspace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadFileRequest {
    /// Path relative to the workspace root.
    pub path: String,
    /// If set, only read `max_bytes` starting at byte `offset`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

/// Result of a [`ReadFileRequest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadFileResult {
    pub path: String,
    pub content: String,
    /// Whether content was truncated by `max_bytes`.
    pub truncated: bool,
}

/// Write a file inside the workspace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteFileRequest {
    /// Path relative to the workspace root.
    pub path: String,
    pub content: String,
    /// Create parent directories if missing.
    #[serde(default = "default_true")]
    pub create_dirs: bool,
}

fn default_true() -> bool { true }

/// Provider-facing interface for workspace-scoped file I/O.
#[async_trait]
pub trait FilesystemCapability: Send + Sync + std::fmt::Debug {
    /// Read a workspace file. The caller has already validated scope.
    async fn read_file(
        &self,
        path: &Path,
        offset: Option<u64>,
        max_bytes: Option<u64>,
    ) -> anyhow::Result<ReadFileResult>;

    /// Write a workspace file. The caller has already validated scope.
    async fn write_file(
        &self,
        path: &Path,
        content: &str,
        create_dirs: bool,
    ) -> anyhow::Result<()>;

    /// List directory contents. The caller has already validated scope.
    async fn list_dir(&self, path: &Path) -> anyhow::Result<Vec<String>>;

    /// Read a named secret from the host's keychain or secrets store.
    /// Returns `Err` if the secret doesn't exist or access is denied.
    async fn read_secret(&self, name: &str) -> anyhow::Result<String>;
}

// ── LocalFilesystem ───────────────────────────────────────────────────────────────

/// Filesystem capability backed by the local OS filesystem via `tokio::fs`.
///
/// Path scope enforcement is handled upstream by the dispatcher;
/// every `path` argument here is already an absolute, in-scope path.
#[derive(Clone, Debug, Default)]
pub struct LocalFilesystem;

impl LocalFilesystem {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FilesystemCapability for LocalFilesystem {
    async fn read_file(
        &self,
        path: &Path,
        offset: Option<u64>,
        max_bytes: Option<u64>,
    ) -> anyhow::Result<ReadFileResult> {
        let path_str = path.display().to_string();

        if offset.is_some() || max_bytes.is_some() {
            let mut file = tokio::fs::File::open(path).await?;
            if let Some(off) = offset {
                file.seek(SeekFrom::Start(off)).await?;
            }
            let limit = max_bytes.unwrap_or(u64::MAX);
            let mut buf = Vec::new();
            file.take(limit).read_to_end(&mut buf).await?;
            let truncated = max_bytes.map(|m| buf.len() as u64 >= m).unwrap_or(false);
            return Ok(ReadFileResult {
                path: path_str,
                content: String::from_utf8_lossy(&buf).into_owned(),
                truncated,
            });
        }

        let content = tokio::fs::read_to_string(path).await?;
        Ok(ReadFileResult {
            path: path_str,
            content,
            truncated: false,
        })
    }

    async fn write_file(
        &self,
        path: &Path,
        content: &str,
        create_dirs: bool,
    ) -> anyhow::Result<()> {
        if create_dirs {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    async fn list_dir(&self, path: &Path) -> anyhow::Result<Vec<String>> {
        let mut reader = tokio::fs::read_dir(path).await?;
        let mut names = Vec::new();
        while let Some(entry) = reader.next_entry().await? {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    /// Read a named secret from the process environment.
    ///
    /// Environment variables are the simplest self-contained secret source
    /// (CI, container, and launchd plists all support them). Replace with a
    /// custom implementation for keychain integration.
    async fn read_secret(&self, name: &str) -> anyhow::Result<String> {
        std::env::var(name)
            .map_err(|_| anyhow::anyhow!("secret not found in environment: {name}"))
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_resolve_in_bounds() {
        let scope = WorkspaceScope::new("/workspace");
        let path = scope.resolve("src/main.rs").unwrap();
        assert_eq!(path, PathBuf::from("/workspace/src/main.rs"));
    }

    #[test]
    fn scope_resolve_traversal_rejected() {
        let scope = WorkspaceScope::new("/workspace");
        let err = scope.resolve("../../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("escapes workspace root"));
    }

    #[test]
    fn scope_resolve_dotdot_within_root_allowed() {
        let scope = WorkspaceScope::new("/workspace");
        let path = scope.resolve("src/../README.md").unwrap();
        assert_eq!(path, PathBuf::from("/workspace/README.md"));
    }

    #[test]
    fn scope_resolve_absolute_escape_rejected() {
        let scope = WorkspaceScope::new("/workspace");
        let err = scope.resolve("/etc/hosts").unwrap_err();
        assert!(err.to_string().contains("escapes workspace root"));
    }
}
