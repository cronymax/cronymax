// Service module — background service management for scheduled task execution.
#![allow(dead_code)]

pub mod platform;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Status of the background service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Unknown,
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceStatus::Running => write!(f, "Running"),
            ServiceStatus::Stopped => write!(f, "Stopped"),
            ServiceStatus::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Manages the background service lifecycle.
///
/// When running as a service, the application keeps a tokio runtime alive
/// for scheduled tasks without creating a window.
pub struct ServiceManager {
    /// Whether the service is currently running.
    running: Arc<AtomicBool>,
    /// Path to the IPC socket for inter-process communication.
    socket_path: PathBuf,
    /// PID file path for process tracking.
    pid_path: PathBuf,
}

impl ServiceManager {
    /// Create a new service manager with default paths.
    pub fn new() -> Self {
        let config_dir = crate::renderer::platform::config_dir();
        Self {
            running: Arc::new(AtomicBool::new(false)),
            socket_path: config_dir.join("service.sock"),
            pid_path: config_dir.join("service.pid"),
        }
    }

    /// Start the background service.
    ///
    /// Creates a PID file and marks the service as running.
    /// The actual event loop / scheduler should be started by the caller.
    pub fn start(&self) -> anyhow::Result<()> {
        if self.is_running() {
            anyhow::bail!("Service is already running");
        }

        // Write PID file.
        let pid = std::process::id();
        if let Some(parent) = self.pid_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.pid_path, pid.to_string())?;

        self.running.store(true, Ordering::SeqCst);
        log::info!("Service started (PID: {})", pid);
        Ok(())
    }

    /// Stop the background service.
    pub fn stop(&self) -> anyhow::Result<()> {
        self.running.store(false, Ordering::SeqCst);

        // Remove PID file.
        if self.pid_path.exists() {
            std::fs::remove_file(&self.pid_path)?;
        }

        // Remove socket file.
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        log::info!("Service stopped");
        Ok(())
    }

    /// Check the status of the service.
    pub fn status(&self) -> ServiceStatus {
        if self.running.load(Ordering::SeqCst) {
            return ServiceStatus::Running;
        }

        // Check PID file for external process.
        if self.pid_path.exists() {
            if let Ok(pid_str) = std::fs::read_to_string(&self.pid_path)
                && let Ok(pid) = pid_str.trim().parse::<u32>()
            {
                // Check if process with this PID exists.
                if process_exists(pid) {
                    return ServiceStatus::Running;
                }
            }
            // Stale PID file — clean up.
            let _ = std::fs::remove_file(&self.pid_path);
        }

        ServiceStatus::Stopped
    }

    /// Whether the service is currently running.
    pub fn is_running(&self) -> bool {
        self.status() == ServiceStatus::Running
    }

    /// Get a clone of the running flag for sharing with background tasks.
    pub fn running_flag(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    /// Get the IPC socket path.
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a process with the given PID exists.
fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks if process exists without sending a signal.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix platforms, assume the process exists if we can't check.
        let _ = pid;
        false
    }
}
