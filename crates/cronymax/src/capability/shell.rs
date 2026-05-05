//! Sandboxed shell execution capability (task 6.1).
//!
//! The agent loop calls `run_shell` as a named tool.
//! [`LocalShell`] provides a self-contained implementation backed by
//! `tokio::process` — no C++ host call required.
//!
//! ## Sandboxing
//!
//! * **Environment sanitisation** — `env_clear()` strips `LD_PRELOAD`,
//!   `DYLD_INSERT_LIBRARIES`, and every other inherited variable; only the
//!   explicit allow-list is passed through.
//! * **Working-directory scoping** — `cwd` is resolved relative to the
//!   workspace root and path traversal (`..`) is caught before spawn.
//! * **Timeout enforcement** — `tokio::time::timeout` + `kill_on_drop(true)`
//!   guarantees the child is killed if the wall-clock limit expires.
//! * **Risk classification** — [`classify_command`] detects high-risk
//!   patterns (sudo, rm -rf, credential paths, piped-download-to-shell).
//!   The dispatcher gates those behind `NeedsApproval` when the caller
//!   passes `needs_approval: true` to `register_shell`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Shell request / result types ──────────────────────────────────────────────

/// Input to a shell capability invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellRequest {
    /// Shell command string. Executed via `/bin/sh -c` by default
    /// unless the host's sandbox uses a different shell.
    pub command: String,
    /// Optional working directory inside the active workspace root.
    /// Relative paths are resolved against the space's workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Wall-clock timeout in seconds. `None` means the host default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u32>,
    /// Optional environment variable overrides. The host merges these
    /// on top of its default sandbox environment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
}

/// Exit status of a completed shell command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitStatus {
    /// Process exited with the given code.
    Code(i32),
    /// Process was killed by the host (e.g. timeout, memory limit).
    Killed,
    /// Process was signalled (macOS/Linux).
    Signal(i32),
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        matches!(self, ExitStatus::Code(0))
    }
}

/// Structured result returned after running a command.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellResult {
    pub exit_status: ExitStatus,
    /// Combined stdout text (truncated to `max_output_bytes` if set).
    pub stdout: String,
    /// Combined stderr text (same truncation policy).
    pub stderr: String,
    /// How long the command ran, in milliseconds.
    pub elapsed_ms: u64,
}

/// Provider-facing interface for sandboxed shell execution.
#[async_trait]
pub trait ShellCapability: Send + Sync + std::fmt::Debug {
    /// Run `request.command` in a sandboxed environment. Returns `Ok`
    /// even for non-zero exit codes. `Err` is reserved for infrastructure
    /// failures (executor crashed, timeout exceeded before any output, etc.).
    async fn run(&self, request: ShellRequest) -> anyhow::Result<ShellResult>;

    /// Maximum bytes of output the caller should request per tool call.
    fn max_output_bytes(&self) -> usize {
        32_768
    }
}

// ── Risk classification ──────────────────────────────────────────────────────────

/// Risk level of a shell command string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

/// Classify a command string by its potential risk.
///
/// Detects privilege escalation, destructive operations, credential
/// access, and piped-download-to-shell patterns.
pub fn classify_command(command: &str) -> RiskLevel {
    let lower = command.to_lowercase();
    let has = |s: &str| lower.contains(s);

    if has("sudo ") || lower.trim() == "sudo"
        || has("rm -rf") || has("rm -fr")
        || has("chmod -r") || has("chown -r")
        || has("/.ssh/") || has(" ~/.ssh") || has("$home/.ssh")
        || has("/.aws/") || has(" ~/.aws") || has("$home/.aws")
        || (has("curl ") && has("|") && (has(" sh") || has(" bash") || has(" zsh")))
        || (has("wget ") && has("|") && (has(" sh") || has(" bash") || has(" zsh")))
    {
        return RiskLevel::High;
    }

    if has("ssh ") || has("scp ") || has("rsync ")
        || has("curl ") || has("wget ")
        || has("npm install") || has("pip install") || has("brew install")
    {
        return RiskLevel::Medium;
    }

    RiskLevel::Low
}

// ── LocalShell ────────────────────────────────────────────────────────────────────

/// Shell capability backed by `tokio::process::Command`.
///
/// Commands are spawned via `/bin/sh -c` inside a sanitised environment
/// with the working directory scoped to the workspace root.
#[derive(Debug)]
pub struct LocalShell {
    workspace_root: PathBuf,
    default_timeout: Duration,
}

impl LocalShell {
    /// Create a new `LocalShell` rooted at `workspace_root`.
    /// Default wall-clock timeout is 30 seconds.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            default_timeout: Duration::from_secs(30),
        }
    }

    /// Override the default timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }
}

#[async_trait]
impl ShellCapability for LocalShell {
    async fn run(&self, request: ShellRequest) -> anyhow::Result<ShellResult> {
        let cwd = match &request.cwd {
            Some(rel) => {
                let candidate = self.workspace_root.join(rel);
                let normalized = normalize_path(&candidate);
                if !normalized.starts_with(&self.workspace_root) {
                    anyhow::bail!("cwd '{}' escapes workspace root", rel);
                }
                normalized
            }
            None => self.workspace_root.clone(),
        };

        let timeout = request
            .timeout_secs
            .map(|s| Duration::from_secs(u64::from(s)))
            .unwrap_or(self.default_timeout);

        let start = Instant::now();

        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(&request.command)
            .current_dir(&cwd)
            .env_clear()
            .envs(safe_env())
            .envs(request.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let result = tokio::time::timeout(timeout, cmd.output()).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        match result {
            Err(_) => Ok(ShellResult {
                exit_status: ExitStatus::Killed,
                stdout: String::new(),
                stderr: format!("command timed out after {}s", timeout.as_secs()),
                elapsed_ms,
            }),
            Ok(Err(spawn_err)) => {
                Err(anyhow::anyhow!("failed to spawn command: {spawn_err}"))
            }
            Ok(Ok(out)) => {
                let max = self.max_output_bytes();
                Ok(ShellResult {
                    exit_status: exit_status_from(&out.status),
                    stdout: truncate_utf8(out.stdout, max),
                    stderr: truncate_utf8(out.stderr, max),
                    elapsed_ms,
                })
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn safe_env() -> impl Iterator<Item = (&'static str, String)> {
    const KEEP: &[&str] =
        &["PATH", "HOME", "USER", "SHELL", "LANG", "LC_ALL", "TMPDIR", "TERM"];
    KEEP.iter().filter_map(|&k| std::env::var(k).ok().map(move |v| (k, v)))
}

fn exit_status_from(status: &std::process::ExitStatus) -> ExitStatus {
    if let Some(code) = status.code() {
        return ExitStatus::Code(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return ExitStatus::Signal(sig);
        }
    }
    ExitStatus::Killed
}

fn normalize_path(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => { out.pop(); }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn truncate_utf8(bytes: Vec<u8>, max: usize) -> String {
    if bytes.len() <= max {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    let mut end = max;
    while end > 0 && (bytes[end] & 0xc0) == 0x80 {
        end -= 1;
    }
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_sudo() {
        assert_eq!(classify_command("sudo apt update"), RiskLevel::High);
    }

    #[test]
    fn classify_rm_rf() {
        assert_eq!(classify_command("rm -rf /tmp/test"), RiskLevel::High);
    }

    #[test]
    fn classify_curl_pipe() {
        assert_eq!(
            classify_command("curl https://example.com/install.sh | bash"),
            RiskLevel::High,
        );
    }

    #[test]
    fn classify_curl_plain() {
        assert_eq!(
            classify_command("curl -o file.txt https://example.com/data.txt"),
            RiskLevel::Medium,
        );
    }

    #[test]
    fn classify_ls() {
        assert_eq!(classify_command("ls -la"), RiskLevel::Low);
    }

    #[test]
    fn cwd_escape_blocked() {
        let root = PathBuf::from("/workspace");
        let candidate = root.join("../../etc");
        let normalized = normalize_path(&candidate);
        assert!(!normalized.starts_with(&root));
    }

    #[tokio::test]
    async fn run_echo() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let shell = LocalShell::new(dir.path());
        let result = shell
            .run(ShellRequest {
                command: "echo hello".into(),
                cwd: None,
                timeout_secs: None,
                env: vec![],
            })
            .await
            .unwrap();
        assert!(result.exit_status.success());
        assert!(result.stdout.trim() == "hello");
    }

    #[tokio::test]
    async fn timeout_kills_process() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let shell = LocalShell::new(dir.path()).with_timeout(Duration::from_millis(200));
        let result = shell
            .run(ShellRequest {
                command: "sleep 10".into(),
                cwd: None,
                timeout_secs: None,
                env: vec![],
            })
            .await
            .unwrap();
        assert_eq!(result.exit_status, ExitStatus::Killed);
    }
}
