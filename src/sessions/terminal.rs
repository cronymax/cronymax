// Terminal session data — terminal PTY session model derived from Session.
//
// Manages the persistent metadata of a terminal session:
// - Shell type and working directory
// - SSH connection state (for remote sessions)
// - Command history reference

use serde::{Deserialize, Serialize};

use super::{Session, SessionType};

/// Terminal session data model — persistent metadata for a PTY session.
///
/// The live PTY state (TermState, pty_writer, etc.) is managed by
/// [`crate::renderer::terminal::TerminalSession`]. This struct holds the
/// serializable/persistent data that survives app restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSessionData {
    /// Base session (profile linkage, timestamps, ID).
    pub session: Session,
    /// Shell program (e.g. "/bin/zsh", "/bin/bash").
    #[serde(default)]
    pub shell: String,
    /// Last known working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// SSH connection info (if this is a remote session).
    #[serde(default)]
    pub ssh: Option<SshState>,
    /// Whether the terminal process has exited.
    #[serde(default)]
    pub exited: bool,
}

/// SSH connection conservation state — persisted for session restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshState {
    /// Remote host (e.g. "user@host.example.com").
    pub host: String,
    /// Remote port (default 22).
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// Remote working directory at time of disconnect.
    #[serde(default)]
    pub remote_cwd: Option<String>,
    /// Whether the connection was active at time of snapshot.
    #[serde(default)]
    pub connected: bool,
}

fn default_ssh_port() -> u16 {
    22
}

impl TerminalSessionData {
    /// Create a new terminal session data for the given profile.
    pub fn new(profile_id: &str, shell: &str) -> Self {
        Self {
            session: Session::new(profile_id, SessionType::Terminal, "Terminal"),
            shell: shell.to_string(),
            cwd: None,
            ssh: None,
            exited: false,
        }
    }
}
