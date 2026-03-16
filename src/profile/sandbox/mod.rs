#![allow(dead_code)]
// Sandbox module — OS-native process sandboxing.

pub mod platform;
pub mod policy;

pub use policy::SandboxPolicy;

/// Apply platform-specific sandbox restrictions before spawning a child process.
///
/// Platform dispatch:
/// - Linux: landlock + seccompiler (stub)
/// - macOS: Seatbelt SBPL via sandbox-exec
/// - Windows: AppContainer (stub)
/// - Other: no-op
pub fn apply_sandbox_pre_exec(policy: &SandboxPolicy) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        platform::linux::apply_sandbox(policy)?;
    }

    #[cfg(target_os = "macos")]
    {
        // macOS sandbox is applied via sandbox-exec, not pre-exec.
        // See platform::macos::spawn_sandboxed_pty_macos().
        let _ = policy;
        log::debug!("macOS sandbox applied via sandbox-exec at PTY spawn time");
    }

    #[cfg(target_os = "windows")]
    {
        platform::windows::spawn_sandboxed_windows("", policy)?;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = policy;
        log::warn!("No sandbox implementation for this platform");
    }

    Ok(())
}
