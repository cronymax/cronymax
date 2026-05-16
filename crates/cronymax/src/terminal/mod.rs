//! PTY-backed interactive terminal session.
//!
//! [`PtySession`] provides a PTY attached to a real shell process — output
//! is raw terminal bytes (VT/ANSI sequences included), which the renderer
//! passes to its terminal emulator. This is the Rust equivalent of
//! `app/terminal/PtySession`.
//!
//! ## Architecture
//!
//! * `PtySession::start()` opens a PTY pair via [`portable_pty`], spawns
//!   the requested shell inside it, and drives reading on a background
//!   `spawn_blocking` task.
//! * Callers receive output through an `mpsc` channel and the exit code
//!   through a `oneshot` channel.
//! * `write()` and `resize()` are synchronous operations on the master fd.
//!
//! ## Thread-safety
//!
//! `PtySession` is `Send`. The underlying master PTY writer is guarded by a
//! `Mutex`; all public methods are safe to call from any thread or task.

pub mod pty;
pub use pty::{PtySessionManager, SharedPtySessionManager};

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::Context as _;
use parking_lot::Mutex;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem as _};
use tokio::sync::{mpsc, oneshot};

/// An active PTY session attached to a shell process.
///
/// Dropping the session does **not** kill the child automatically — call
/// [`PtySession::stop()`] explicitly to terminate the process.
pub struct PtySession {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    pid: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("pid", &self.pid.load(Ordering::Relaxed))
            .field("running", &self.running.load(Ordering::Relaxed))
            .finish()
    }
}

// `portable_pty`'s `Write` trait object is not `Debug` so we use a wrapper
// alias to satisfy the derive.
trait Write: std::io::Write {}
impl<T: std::io::Write> Write for T {}

impl PtySession {
    /// Spawn a new PTY session.
    ///
    /// # Arguments
    ///
    /// * `cwd`        — initial working directory for the shell.
    /// * `shell`      — path to the shell binary (e.g. `/bin/zsh`).
    /// * `cols`/`rows`— initial terminal dimensions.
    /// * `output_tx`  — receives raw output bytes from the child.
    /// * `exit_tx`    — fired once with the exit code when the child dies.
    pub async fn start(
        cwd: &Path,
        shell: &str,
        cols: u16,
        rows: u16,
        output_tx: mpsc::UnboundedSender<Vec<u8>>,
        exit_tx: oneshot::Sender<i32>,
    ) -> anyhow::Result<Self> {
        let cwd = cwd.to_owned();
        let shell = shell.to_owned();

        // PTY operations are blocking; run them on the blocking thread pool.
        tokio::task::spawn_blocking(move || {
            Self::start_blocking(cwd, shell, cols, rows, output_tx, exit_tx)
        })
        .await
        .context("PTY spawn task panicked")?
    }

    fn start_blocking(
        cwd: std::path::PathBuf,
        shell: String,
        cols: u16,
        rows: u16,
        output_tx: mpsc::UnboundedSender<Vec<u8>>,
        exit_tx: oneshot::Sender<i32>,
    ) -> anyhow::Result<Self> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to open PTY pair")?;

        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(&cwd);

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .context("failed to spawn shell")?;
        // Drop the slave end in this process — the child now owns it.
        drop(pair.slave);

        let pid = child.process_id().unwrap_or(0);

        let master = Arc::new(Mutex::new(pair.master));
        let writer: Box<dyn Write + Send> = {
            let w = master
                .lock()
                .take_writer()
                .context("failed to get PTY writer")?;
            Box::new(w)
        };
        let writer = Arc::new(Mutex::new(writer));
        let running = Arc::new(AtomicBool::new(true));
        let pid_atomic = Arc::new(AtomicU32::new(pid));

        // Reader thread: forward raw PTY output to the channel.
        {
            let reader = master
                .lock()
                .try_clone_reader()
                .context("failed to clone PTY reader")?;
            let running_clone = Arc::clone(&running);
            let output_tx_clone = output_tx;
            std::thread::spawn(move || {
                let mut reader = reader;
                let mut buf = vec![0u8; 4096];
                loop {
                    match std::io::Read::read(&mut reader, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let _ = output_tx_clone.send(buf[..n].to_vec());
                        }
                    }
                }
                running_clone.store(false, Ordering::Relaxed);
            });
        }

        // Waiter thread: block on child exit and fire the oneshot.
        {
            let running_clone = Arc::clone(&running);
            std::thread::spawn(move || {
                let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
                running_clone.store(false, Ordering::Relaxed);
                let _ = exit_tx.send(code);
            });
        }

        Ok(Self {
            writer,
            master,
            pid: pid_atomic,
            running,
        })
    }

    /// Write raw bytes to the PTY master (i.e. the child's stdin).
    pub fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        self.writer
            .lock()
            .write_all(data)
            .context("PTY write failed")
    }

    /// Resize the terminal.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master
            .lock()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("PTY resize failed")
    }

    /// Send SIGHUP to the child process to request termination.
    pub fn stop(&self) {
        if self.running.load(Ordering::Relaxed) {
            let pid = self.pid.load(Ordering::Relaxed);
            if pid != 0 {
                // SAFETY: nix::sys::signal is not callable from forbid-unsafe
                // code, but std::process::Command::kill() is not available for
                // arbitrary pids. We use libc via nix's safe wrapper.
                // nix is forbidden here; use kill(2) through std instead.
                // portable-pty's Child::kill() is the correct path but we only
                // have the pid here. Use the OS-provided approach via a
                // one-shot blocking task.
                let _ = std::process::Command::new("kill")
                    .arg("-HUP")
                    .arg(pid.to_string())
                    .status();
            }
        }
    }

    /// Returns `true` while the child process is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Returns the child PID, or 0 if unavailable.
    pub fn pid(&self) -> u32 {
        self.pid.load(Ordering::Relaxed)
    }
}
