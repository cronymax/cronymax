pub mod input;
pub mod links;
pub mod pty;
pub mod state;

use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;

use crate::renderer::terminal::pty::Pty;
use crate::renderer::terminal::state::TermState;

/// Unique session identifier.
pub type SessionId = u32;

/// Grid dimensions in columns × rows.
#[derive(Debug, Clone, Copy)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

/// A single terminal session: PTY + state machine + metadata.
pub struct TerminalSession {
    pub id: SessionId,
    pub state: TermState,
    pub pty_writer: Box<dyn Write + Send>,
    pub title: String,
    pub grid_size: GridSize,
    pub is_dirty: bool,
    /// Channel receiving raw PTY bytes from the reader thread.
    pub pty_rx: mpsc::Receiver<Vec<u8>>,
    /// Whether the shell has exited.
    pub exited: bool,
    /// PID of the child shell process (for CWD queries).
    pub child_pid: Option<u32>,
    /// Current working directory of the shell (updated on PTY output).
    pub cwd: Option<String>,
    /// Keep the PTY master alive for the session lifetime.
    /// On Windows ConPty, dropping the master destroys the pseudo-console
    /// and kills the child process.
    _pty_master: Box<dyn portable_pty::MasterPty + Send>,
    /// Keep the child handle alive so the OS doesn't terminate the process.
    _pty_child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl TerminalSession {
    /// Create a new terminal session, spawning a shell in a PTY.
    /// If a sandbox policy is provided, the shell process is confined by
    /// OS-level sandbox rules (Seatbelt on macOS).
    pub fn new(
        id: SessionId,
        shell: &str,
        cols: u16,
        rows: u16,
        scrollback: usize,
        sandbox: Option<&crate::profile::sandbox::policy::SandboxPolicy>,
        event_proxy: Option<winit::event_loop::EventLoopProxy<crate::ai::stream::AppEvent>>,
    ) -> Self {
        let pty = Pty::spawn(shell, cols, rows, sandbox);
        let state = TermState::new(cols as usize, rows as usize, scrollback);

        // Spawn a reader thread that sends PTY output bytes to the main thread.
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut reader = pty.reader;
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // PTY closed (shell exited).
                        let _ = tx.send(Vec::new());
                        if let Some(ref proxy) = event_proxy {
                            let _ = proxy.send_event(crate::ai::stream::AppEvent::PtyDataReady {
                                session_id: id,
                            });
                        }
                        break;
                    }
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        if let Some(ref proxy) = event_proxy {
                            let _ = proxy.send_event(crate::ai::stream::AppEvent::PtyDataReady {
                                session_id: id,
                            });
                        }
                    }
                    Err(e) => {
                        log::error!("PTY read error: {}", e);
                        // On Windows ConPTY, a read error typically means the
                        // child process has terminated.  Signal exit the same
                        // way as EOF so `process_pty_output` sets `exited`.
                        let _ = tx.send(Vec::new());
                        if let Some(ref proxy) = event_proxy {
                            let _ = proxy.send_event(crate::ai::stream::AppEvent::PtyDataReady {
                                session_id: id,
                            });
                        }
                        break;
                    }
                }
            }
        });

        let child_pid = pty.child.process_id();

        Self {
            id,
            state,
            pty_writer: pty.writer,
            title: String::from("cronymax"),
            grid_size: GridSize { cols, rows },
            is_dirty: true,
            pty_rx: rx,
            exited: false,
            child_pid,
            cwd: None,
            _pty_master: pty.master,
            _pty_child: pty.child,
        }
    }

    /// Drain all pending PTY bytes and feed them to the terminal state.
    /// Returns true if any bytes were processed.
    pub fn process_pty_output(&mut self) -> bool {
        let mut processed = false;
        while let Ok(bytes) = self.pty_rx.try_recv() {
            if bytes.is_empty() {
                self.exited = true;
                log::info!("Session {} shell exited", self.id);
                break;
            }
            self.state.advance(&bytes);
            processed = true;
        }
        // On Windows ConPTY the reader thread may block indefinitely even
        // after the child exits.  Fall back to checking the child process
        // status directly so callers always see `exited == true` once the
        // shell terminates.
        if !self.exited {
            if let Ok(Some(_status)) = self._pty_child.try_wait() {
                self.exited = true;
                log::info!("Session {} child process exited", self.id);
            }
        }
        if processed {
            self.is_dirty = true;
            // Update title from terminal state if available, else from CWD.
            if let Some(new_title) = self.state.title() {
                self.title = new_title;
            } else {
                self.update_title_from_cwd();
            }
        }
        processed
    }

    /// Query the child process CWD and set the tab title to a shortened path.
    fn update_title_from_cwd(&mut self) {
        if let Some(pid) = self.child_pid
            && let Some(cwd) = Self::query_cwd(pid)
        {
            self.title = Self::shorten_path(&cwd);
            self.cwd = Some(cwd);
        }
    }

    /// Query the current working directory of a process by PID.
    #[cfg(target_os = "macos")]
    pub fn query_cwd(pid: u32) -> Option<String> {
        use std::process::Command;
        let output = Command::new("lsof")
            .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        // lsof output: lines starting with 'n' contain the path
        stdout
            .lines()
            .find(|line| line.starts_with('n') && line.len() > 1)
            .map(|line| line[1..].to_string())
    }

    #[cfg(target_os = "linux")]
    pub fn query_cwd(pid: u32) -> Option<String> {
        std::fs::read_link(format!("/proc/{}/cwd", pid))
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn query_cwd(_pid: u32) -> Option<String> {
        None
    }

    /// Shorten a path for display: replace $HOME with ~, show last 2 components
    /// when path is long.
    fn shorten_path(path: &str) -> String {
        let home = std::env::var("HOME").unwrap_or_default();
        let display = if !home.is_empty() && path.starts_with(&home) {
            format!("~{}", &path[home.len()..])
        } else {
            path.to_string()
        };
        // If path is short enough, display as-is.
        if display.len() <= 30 {
            return display;
        }
        // Otherwise, show ~/…/last_two_components
        let parts: Vec<&str> = display.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() <= 2 {
            return display;
        }
        let prefix = if display.starts_with('~') { "~" } else { "" };
        format!("{}/…/{}", prefix, parts[parts.len() - 2..].join("/"))
    }

    /// Write raw bytes to the PTY (keyboard input).
    pub fn write_to_pty(&mut self, data: &[u8]) {
        // Snap viewport to bottom so the user sees the cursor/prompt.
        self.state.scroll_to_bottom();

        if let Err(e) = self.pty_writer.write_all(data) {
            log::error!("PTY write error: {}", e);
            return;
        }
        // Flush immediately so data reaches the ConPty without waiting
        // for the write buffer to fill.  This is essential on Windows
        // where the pipe writer may buffer small writes.
        if let Err(e) = self.pty_writer.flush() {
            log::error!("PTY flush error: {}", e);
        }
    }

    /// Resize the terminal session (PTY + state + ConPty).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.grid_size = GridSize { cols, rows };
        self.state.resize(cols as usize, rows as usize);
        self.is_dirty = true;
        self._pty_master
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .ok();
    }
}
