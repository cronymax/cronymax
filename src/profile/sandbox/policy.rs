#![allow(dead_code)]
// Sandbox policy definitions — FsPolicy, NetworkPolicy, SandboxPolicy.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Filesystem access policy for child shell processes.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct FsPolicy {
    /// Paths the shell may read (recursive).
    pub read_allow: Vec<String>,
    /// Paths the shell may write. Should be a subset of read_allow.
    pub write_allow: Vec<String>,
    /// Explicit deny overrides (applied after allow rules).
    pub deny: Vec<String>,
}

/// Network access policy for child shell processes.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct NetworkPolicy {
    /// true = deny all outbound connections by default.
    pub default_deny: bool,
    /// Allowed outbound connections (host:port or host:*).
    pub allow_outbound: Vec<String>,
    /// Denied outbound connections.
    pub deny_outbound: Vec<String>,
}

/// Combined sandbox policy loaded from policy.toml.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct SandboxPolicy {
    pub fs: FsPolicy,
    pub network: NetworkPolicy,
}

impl SandboxPolicy {
    /// Load a sandbox policy from a TOML file.
    pub fn from_toml_file(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let policy: SandboxPolicy = toml::from_str(&contents)?;
        Ok(policy)
    }

    /// Create a permissive default policy (no restrictions).
    pub fn from_default() -> Self {
        SandboxPolicy {
            fs: FsPolicy {
                read_allow: vec!["~".into(), "/usr".into(), "/etc".into(), "/tmp".into()],
                write_allow: vec!["~".into(), "/tmp".into()],
                deny: vec!["~/.ssh".into(), "~/.gnupg".into()],
            },
            network: NetworkPolicy {
                default_deny: false,
                allow_outbound: vec![],
                deny_outbound: vec![],
            },
        }
    }

    /// Serialize the policy to a TOML string.
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Save the policy to a TOML file.
    pub fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        let contents = self.to_toml_string()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Paths that are always implicitly allowed for PTY operation.
    pub fn always_allowed_paths() -> &'static [&'static str] {
        &[
            "/dev/ptmx",
            "/dev/pts",
            "/dev/tty",
            "/dev/null",
            "/dev/urandom",
        ]
    }

    /// Expand `~` tokens to the user's home directory.
    pub fn expand_path(path: &str) -> PathBuf {
        if (path == "~" || path.starts_with("~/"))
            && let Some(home) = dirs::home_dir()
        {
            return home.join(path.strip_prefix("~/").unwrap_or(""));
        }
        PathBuf::from(path)
    }

    /// Check whether a shell command string references any denied filesystem paths.
    ///
    /// Returns `Ok(())` if the command is allowed, or `Err(reason)` if it
    /// touches a denied path.  This is a best-effort heuristic — it expands
    /// `~` and `$HOME` references and checks if any token in the command
    /// resolves to (or is a prefix/sub-path of) a denied path.
    pub fn check_command(&self, command: &str) -> Result<(), String> {
        if self.fs.deny.is_empty() {
            return Ok(());
        }

        let home_str = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default();

        // Expand the denied paths once.
        let denied_expanded: Vec<PathBuf> =
            self.fs.deny.iter().map(|p| Self::expand_path(p)).collect();

        // Normalise the command: expand ~ and $HOME so path matching works on
        // absolute paths.  We check every whitespace-delimited "token" as well
        // as common shell patterns (quotes, command substitution contents).
        let normalised = command
            .replace("$HOME", &home_str)
            .replace("${HOME}", &home_str);
        // Also expand a bare ~ that is followed by / or is standalone.
        let normalised = Self::expand_tilde_in_string(&normalised, &home_str);

        for denied in &denied_expanded {
            let denied_str = denied.to_string_lossy();

            // Check each whitespace-delimited token.
            for token in normalised.split_whitespace() {
                // Strip surrounding quotes.
                let token = token
                    .trim_start_matches(['\'', '"'])
                    .trim_end_matches(['\'', '"']);
                if token.is_empty() {
                    continue;
                }

                // Check if this token is a path that falls under a denied subtree.
                let token_path = Path::new(token);
                if token_path.starts_with(denied) || denied.starts_with(token_path) {
                    return Err(format!(
                        "Sandbox policy denies access to '{}' (denied path: {})",
                        token, denied_str
                    ));
                }
            }
        }

        Ok(())
    }

    /// Expand `~` references in a command string to the home directory.
    fn expand_tilde_in_string(s: &str, home: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '~' {
                // ~ followed by / or whitespace or end-of-string → expand
                match chars.peek() {
                    None | Some('/') | Some(' ') | Some('\t') | Some('"') | Some('\'') => {
                        result.push_str(home);
                    }
                    _ => {
                        // ~user or other — don't expand
                        result.push(ch);
                    }
                }
            } else {
                result.push(ch);
            }
        }
        result
    }
}
