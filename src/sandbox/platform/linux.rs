// Linux sandbox — landlock + seccompiler.
//
// Landlock FS restrictions and seccompiler BPF network filtering.
// These are compile-gated for Linux only.

use crate::sandbox::policy::{FsPolicy, NetworkPolicy, SandboxPolicy};

/// Apply filesystem sandbox using Landlock v3 ABI.
///
/// This is a stub — actual landlock implementation requires the landlock crate.
/// On kernels < 5.13, this is a no-op.
pub fn apply_fs_sandbox(_policy: &FsPolicy) -> anyhow::Result<()> {
    // TODO: Implement using landlock crate when enabled.
    // For now, log and no-op.
    log::info!("Linux FS sandbox: landlock not yet implemented (stub)");
    Ok(())
}

/// Apply network sandbox using seccompiler BPF.
///
/// This is a stub — actual implementation requires the seccompiler crate.
pub fn apply_net_sandbox(_policy: &NetworkPolicy) -> anyhow::Result<()> {
    // TODO: Implement using seccompiler crate when enabled.
    log::info!("Linux network sandbox: seccompiler not yet implemented (stub)");
    Ok(())
}

/// Apply the full sandbox policy pre-exec.
pub fn apply_sandbox(policy: &SandboxPolicy) -> anyhow::Result<()> {
    apply_fs_sandbox(&policy.fs)?;
    if policy.network.default_deny {
        apply_net_sandbox(&policy.network)?;
    }
    Ok(())
}
