// Windows sandbox — AppContainer skeleton.
//
// Uses Win32 Security AppContainer APIs for process isolation.
// Compile-gated with #[cfg(target_os = "windows")].

use crate::profile::sandbox::policy::SandboxPolicy;

/// Spawn a sandboxed process using Windows AppContainer.
///
/// This is a stub — actual implementation requires the windows crate.
pub fn spawn_sandboxed_windows(_shell: &str, _policy: &SandboxPolicy) -> anyhow::Result<()> {
    // TODO: Implement using windows crate AppContainer APIs.
    log::info!("Windows sandbox: AppContainer not yet implemented (stub)");
    Ok(())
}
