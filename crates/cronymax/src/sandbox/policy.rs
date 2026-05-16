//! [`SandboxPolicy`]: read/write/deny path rules and network flag.
//!
//! Mirrors `app/sandbox/SandboxPolicy`. Path matching uses `starts_with`
//! prefix semantics. Deny paths take priority over read/write allows.

use std::path::{Path, PathBuf};

/// Controls what a sandboxed process may read, write, and access on the
/// network. Constructed per-Space and passed to [`super::PermissionBroker`].
#[derive(Clone, Debug)]
pub struct SandboxPolicy {
    workspace_root: PathBuf,
    read_paths: Vec<PathBuf>,
    write_paths: Vec<PathBuf>,
    deny_paths: Vec<PathBuf>,
    allow_network: bool,
}

impl SandboxPolicy {
    /// Default policy for a workspace root.
    ///
    /// The root itself is both readable and writable; all sub-paths
    /// inherit those rights. Network access is disabled by default.
    pub fn default_for_workspace(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            read_paths: vec![root.clone()],
            write_paths: vec![root.clone()],
            deny_paths: vec![],
            allow_network: false,
            workspace_root: root,
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
    pub fn read_paths(&self) -> &[PathBuf] {
        &self.read_paths
    }
    pub fn write_paths(&self) -> &[PathBuf] {
        &self.write_paths
    }
    pub fn deny_paths(&self) -> &[PathBuf] {
        &self.deny_paths
    }
    pub fn allow_network(&self) -> bool {
        self.allow_network
    }
    pub fn set_allow_network(&mut self, v: bool) {
        self.allow_network = v;
    }

    // ── Mutators ──────────────────────────────────────────────────────────

    pub fn add_read_path(&mut self, path: impl Into<PathBuf>) {
        self.read_paths.push(path.into());
    }
    pub fn add_write_path(&mut self, path: impl Into<PathBuf>) {
        self.write_paths.push(path.into());
    }
    pub fn add_deny_path(&mut self, path: impl Into<PathBuf>) {
        self.deny_paths.push(path.into());
    }

    // ── Permission checks ─────────────────────────────────────────────────

    /// Returns `true` if `path` is within a read-allowed subtree and is not
    /// explicitly denied.
    pub fn can_read(&self, path: &Path) -> bool {
        if self.deny_paths.iter().any(|d| path.starts_with(d)) {
            return false;
        }
        self.read_paths.iter().any(|r| path.starts_with(r))
    }

    /// Returns `true` if `path` is within a write-allowed subtree and is not
    /// explicitly denied.
    pub fn can_write(&self, path: &Path) -> bool {
        if self.deny_paths.iter().any(|d| path.starts_with(d)) {
            return false;
        }
        self.write_paths.iter().any(|w| path.starts_with(w))
    }

    // ── Seatbelt profile ──────────────────────────────────────────────────

    /// Generate a macOS `sandbox-exec` (Seatbelt v1) profile string.
    ///
    /// Produces a deny-all base with targeted allow rules for the
    /// configured paths. Suitable for passing to `sandbox_init(3)`.
    pub fn to_seatbelt_profile(&self) -> String {
        let mut out = String::from("(version 1)\n(deny default)\n");
        for p in &self.read_paths {
            out.push_str(&format!(
                "(allow file-read* (subpath \"{}\"))\n",
                p.display()
            ));
        }
        for p in &self.write_paths {
            out.push_str(&format!(
                "(allow file-write* (subpath \"{}\"))\n",
                p.display()
            ));
        }
        if self.allow_network {
            out.push_str("(allow network*)\n");
        }
        out.push_str("(allow process-exec)\n(allow process-fork)\n");
        out
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_allows_workspace() {
        let p = SandboxPolicy::default_for_workspace("/ws");
        assert!(p.can_read(Path::new("/ws/src/main.rs")));
        assert!(p.can_write(Path::new("/ws/out/result.json")));
    }

    #[test]
    fn deny_overrides_read() {
        let mut p = SandboxPolicy::default_for_workspace("/ws");
        p.add_deny_path("/ws/secrets");
        assert!(!p.can_read(Path::new("/ws/secrets/api_key")));
        assert!(p.can_read(Path::new("/ws/src/main.rs")));
    }

    #[test]
    fn outside_workspace_denied() {
        let p = SandboxPolicy::default_for_workspace("/ws");
        assert!(!p.can_read(Path::new("/etc/passwd")));
        assert!(!p.can_write(Path::new("/usr/local/bin/evil")));
    }

    #[test]
    fn extra_read_path() {
        let mut p = SandboxPolicy::default_for_workspace("/ws");
        p.add_read_path("/shared/lib");
        assert!(p.can_read(Path::new("/shared/lib/header.h")));
    }

    #[test]
    fn seatbelt_profile_deny_default_present() {
        let p = SandboxPolicy::default_for_workspace("/ws");
        let profile = p.to_seatbelt_profile();
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow file-read*"));
    }
}
