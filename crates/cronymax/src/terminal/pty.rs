//! Per-workspace PTY session manager.
//!
//! `SessionManager` maps opaque session IDs to live [`PtySession`]s. It is
//! the Rust counterpart of the per-`Space` PTY bookkeeping in C++
//! (`TerminalSession` struct and `pty->Start()/Write()/Resize()/Stop()`).
//!
//! Output bytes are forwarded to the caller via a provided `broadcast_fn`
//! closure so the GIPS bridge can push `terminal.output` events to the
//! renderer without the manager needing to know about GIPS internals.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::debug;

use crate::terminal::PtySession;

/// A live terminal entry.
struct Entry {
    session: PtySession,
    /// Cwd passed at creation time (stored for diagnostics).
    _cwd: PathBuf,
}

/// Manages a collection of PTY sessions for one workspace.
///
/// All methods require `&mut self`; the caller should hold this in a
/// `tokio::sync::Mutex` when sharing across tasks.
#[derive(Debug, Default)]
pub struct PtySessionManager {
    sessions: HashMap<String, Entry>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("session", &self.session)
            .finish()
    }
}

impl PtySessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a new PTY shell and register it under the given `id`.
    ///
    /// `output_fn` is called (from a background task) each time the PTY
    /// produces output. `exit_fn` is called once when the child exits.
    #[allow(clippy::too_many_arguments)]
    pub async fn create<O, E>(
        &mut self,
        id: String,
        cwd: PathBuf,
        shell: &str,
        cols: u16,
        rows: u16,
        output_fn: O,
        exit_fn: E,
    ) -> anyhow::Result<()>
    where
        O: Fn(Vec<u8>) + Send + 'static,
        E: Fn(i32) + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (exit_tx, mut exit_rx) = oneshot::channel::<i32>();

        let session = PtySession::start(&cwd, shell, cols, rows, out_tx, exit_tx).await?;

        // Forward output bytes to caller via background task.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    Some(chunk) = out_rx.recv() => { output_fn(chunk); }
                    Ok(code) = &mut exit_rx => {
                        // Drain any remaining output.
                        while let Ok(chunk) = out_rx.try_recv() { output_fn(chunk); }
                        exit_fn(code);
                        break;
                    }
                    else => break,
                }
            }
        });

        self.sessions.insert(id, Entry { session, _cwd: cwd });
        Ok(())
    }

    /// Write bytes to the PTY's stdin.
    pub fn write(&self, id: &str, data: &[u8]) -> anyhow::Result<()> {
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))?;
        entry.session.write(data)
    }

    /// Resize the PTY window.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> anyhow::Result<()> {
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))?;
        entry.session.resize(cols, rows)
    }

    /// Stop (kill) a session and remove it.
    pub fn close(&mut self, id: &str) {
        if let Some(entry) = self.sessions.remove(id) {
            entry.session.stop();
            debug!(%id, "terminal session closed");
        }
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.sessions
            .get(id)
            .map(|e| e.session.is_running())
            .unwrap_or(false)
    }

    pub fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }
}

/// Thread-safe wrapper around [`PtySessionManager`].
pub type SharedPtySessionManager = Arc<Mutex<PtySessionManager>>;

impl PtySessionManager {
    pub fn new_shared() -> SharedPtySessionManager {
        Arc::new(Mutex::new(PtySessionManager::new()))
    }
}
