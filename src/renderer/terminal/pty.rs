#![allow(dead_code)]
//! PTY spawning and I/O via portable-pty.

use std::io::{Read, Write};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::profile::sandbox::policy::SandboxPolicy;

/// Wraps a portable-pty master, providing reader/writer access and resize.
pub struct Pty {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Pty {
    /// Spawn a new PTY running the given shell command at the specified grid size.
    /// If a sandbox policy is provided, the shell is wrapped in `sandbox-exec`
    /// on macOS so the child process is confined by OS-level Seatbelt rules.
    pub fn spawn(shell: &str, cols: u16, rows: u16, sandbox: Option<&SandboxPolicy>) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY");

        let cmd = Self::build_command(shell, sandbox);
        let child = pair
            .slave
            .spawn_command(cmd)
            .expect("Failed to spawn shell");

        let reader = pair
            .master
            .try_clone_reader()
            .expect("Failed to clone PTY reader");
        let writer = pair
            .master
            .take_writer()
            .expect("Failed to take PTY writer");

        Self {
            master: pair.master,
            reader,
            writer,
            child,
        }
    }

    /// Try to spawn a PTY. Returns `None` on failure (logs error).
    pub fn try_spawn(
        shell: &str,
        cols: u16,
        rows: u16,
        sandbox: Option<&SandboxPolicy>,
    ) -> Option<Self> {
        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to open PTY: {}", e);
                return None;
            }
        };

        let cmd = Self::build_command(shell, sandbox);
        let child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to spawn shell '{}': {}", shell, e);
                return None;
            }
        };

        let reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to clone PTY reader: {}", e);
                return None;
            }
        };

        let writer = match pair.master.take_writer() {
            Ok(w) => w,
            Err(e) => {
                log::error!("Failed to take PTY writer: {}", e);
                return None;
            }
        };

        Some(Self {
            master: pair.master,
            reader,
            writer,
            child,
        })
    }

    /// Build a `CommandBuilder` for the shell, optionally wrapping it in
    /// `sandbox-exec` on macOS when a sandbox policy is provided.
    fn build_command(shell: &str, sandbox: Option<&SandboxPolicy>) -> CommandBuilder {
        #[cfg(target_os = "macos")]
        if let Some(policy) = sandbox {
            // Generate SBPL profile string and pass inline via -p flag.
            // We use -p instead of -f (temp file) because portable_pty's
            // fork+exec can race with temp-file cleanup.
            let sbpl = crate::profile::sandbox::platform::macos::sbpl_from_policy(policy);
            let mut cmd = CommandBuilder::new("sandbox-exec");
            cmd.env("TERM", "xterm-256color");
            cmd.arg("-p");
            cmd.arg(&sbpl);
            cmd.arg(shell);
            log::info!(
                "PTY sandbox enabled: sandbox-exec -p <{} bytes> {}",
                sbpl.len(),
                shell
            );
            return cmd;
        }

        // Fallback: spawn unsandboxed (also used on non-macOS).
        let _ = sandbox;
        CommandBuilder::new(shell)
    }

    /// Resize the PTY to the given grid dimensions.
    pub fn resize(&self, cols: u16, rows: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .ok();
    }
}
