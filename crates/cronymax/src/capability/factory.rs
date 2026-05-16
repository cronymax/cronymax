//! [`CapabilityFactory`] trait and [`DefaultCapabilityFactory`] implementation.
//!
//! Centralises the wiring of tier-dependent capabilities (shell + filesystem)
//! so call sites never test `if sandbox_policy.is_some()` directly. The
//! factory returns a [`DispatcherBuilder`] pre-loaded with the correct
//! [`ShellCapability`] and [`FilesystemCapability`] for the given
//! [`SandboxTier`]; `AgentRunner` then adds the remaining run-specific
//! capabilities (test_runner, submit_document, flow_tools, etc.).

use std::path::Path;
use std::sync::Arc;

use crate::capability::dispatcher::DispatcherBuilder;
use crate::capability::filesystem::{LocalFilesystem, WorkspaceScope};
use crate::capability::notify::NullNotify;
use crate::capability::shell::LocalShell;
use crate::capability::tier::SandboxTier;
use crate::sandbox::broker::PermissionBroker;
use crate::sandbox::fs_gate::PolicyFilesystem;
use crate::sandbox::shell_gate::PolicyShell;

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Factory for assembling a [`DispatcherBuilder`] with the appropriate
/// sandbox-tier-specific capabilities.
///
/// Implementations choose `LocalShell`/`LocalFilesystem` for
/// [`SandboxTier::Trusted`] and `PolicyShell`/`PolicyFilesystem` for
/// [`SandboxTier::Sandboxed`].
pub trait CapabilityFactory: Send + Sync {
    /// Return a [`DispatcherBuilder`] with shell, filesystem, and notify
    /// registered for `workspace_root` at the given `tier`.
    ///
    /// `AgentRunner` adds run-specific capabilities (submit_document,
    /// test_runner, flow_tools) after this call.
    fn build(&self, workspace_root: &Path, tier: SandboxTier) -> DispatcherBuilder;
}

// ── DefaultCapabilityFactory ──────────────────────────────────────────────────

/// Default implementation of [`CapabilityFactory`].
///
/// * `Trusted` → `LocalShell` + `LocalFilesystem` (unrestricted).
/// * `Sandboxed(policy)` → `PolicyShell` + `PolicyFilesystem` (policy-gated).
#[derive(Clone, Debug, Default)]
pub struct DefaultCapabilityFactory;

impl CapabilityFactory for DefaultCapabilityFactory {
    fn build(&self, workspace_root: &Path, tier: SandboxTier) -> DispatcherBuilder {
        let scope = WorkspaceScope::new(workspace_root);
        let mut builder = DispatcherBuilder::new();

        match tier {
            SandboxTier::Trusted => {
                builder.register_shell(Arc::new(LocalShell::new(workspace_root)), false);
                builder.register_filesystem(Arc::new(LocalFilesystem), scope);
            }
            SandboxTier::Sandboxed(policy) => {
                let broker = PermissionBroker::new();
                let shell = PolicyShell::new(
                    LocalShell::new(workspace_root),
                    broker.clone(),
                    Arc::clone(&policy),
                );
                builder.register_shell(Arc::new(shell), false);
                let fs = PolicyFilesystem::new(LocalFilesystem, broker, Arc::clone(&policy));
                builder.register_filesystem(Arc::new(fs), scope);
            }
        }

        builder.register_notify(Arc::new(NullNotify));
        builder
    }
}

// ── FakeCapabilityFactory ─────────────────────────────────────────────────────

/// Test double that builds a trusted-tier dispatcher (no real policy checks).
/// Used by integration tests via [`crate::llm::MockLlmFactory`].
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Debug, Default)]
pub struct FakeCapabilityFactory;

#[cfg(any(test, feature = "testing"))]
impl CapabilityFactory for FakeCapabilityFactory {
    fn build(&self, workspace_root: &Path, _tier: SandboxTier) -> DispatcherBuilder {
        // Always use trusted capabilities regardless of tier — tests don't
        // want policy gates to interfere with assertions.
        DefaultCapabilityFactory.build(workspace_root, SandboxTier::Trusted)
    }
}
