//! Explicit sandbox tier for capability construction.
//!
//! Replaces the per-call-site `if sandbox_policy.is_some()` conditionals
//! in `handler.rs`. [`CapabilityFactory::build`] branches on this enum
//! so no capability-construction code lives outside the factory.

use std::sync::Arc;

use crate::sandbox::policy::SandboxPolicy;

/// Identifies which capability sandbox tier to apply for a given invocation.
#[derive(Clone, Debug)]
pub enum SandboxTier {
    /// Full trust — capabilities use `LocalShell` and `LocalFilesystem`
    /// with no policy gate. Used for chat turns and entry agents which run
    /// under explicit user action.
    Trusted,
    /// Policy-gated — capabilities use `PolicyShell` and `PolicyFilesystem`
    /// with the provided `SandboxPolicy`. Used for flow sub-agents.
    Sandboxed(Arc<SandboxPolicy>),
}
