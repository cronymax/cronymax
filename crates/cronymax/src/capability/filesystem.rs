//! Filesystem mediation, workspace scope enforcement, and secret
//! access (task 6.3).
//!
//! The agent loop may call `read_file`, `write_file`, and
//! `read_secret`. All paths are validated against the active
//! [`WorkspaceScope`] before the host is asked to perform the I/O.
//! Paths that escape the scope boundary are rejected with a structured
//! error — the host never sees them.
//!
//! ## Workspace scope
//!
//! Every file request is relative to the Space's workspace root
//! (surfaced as `WorkspaceScope::root`). The runtime resolves absolute
//! paths and ensures no `..` traversal exits the root. The host is
//! therefore trusted to perform the I/O but not to validate the scope.
//!
//! ## Secrets
//!
//! Secrets (API keys, tokens, etc.) are read-only from the runtime's
//! perspective; the agent loop can read them, not write them. The host
//! bridges to the system keychain or a dedicated secrets store.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Workspace scope ──────────────────────────────────────────────────────────

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

// ── Read / write file requests ───────────────────────────────────────────────

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

/// Provider-facing interface for workspace-scoped file I/O. The
/// implementation lives in `crony/` and bridges to the host filesystem.
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

// ── Unit tests ───────────────────────────────────────────────────────────────

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
        // A path like `src/../README.md` normalises to `/workspace/README.md`
        let path = scope.resolve("src/../README.md").unwrap();
        assert_eq!(path, PathBuf::from("/workspace/README.md"));
    }

    #[test]
    fn scope_resolve_absolute_escape_rejected() {
        let scope = WorkspaceScope::new("/workspace");
        // Passing an absolute path that's outside the root
        let err = scope.resolve("/etc/hosts").unwrap_err();
        assert!(err.to_string().contains("escapes workspace root"));
    }
}
