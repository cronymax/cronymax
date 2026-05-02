//! Permission policy and brokering for sandboxed execution.
//!
//! Mirrors the `app/sandbox/` subsystem:
//!
//! * [`SandboxPolicy`] — per-Space read/write/deny path rules and network flag.
//! * [`PermissionBroker`] — stateless evaluator: `check_exec`, `check_read`,
//!   `check_write` against a policy.
//! * [`Actor`] — the principal making the request (User or Agent).
//! * [`PermissionDecision`] — the outcome: allowed / requires_confirmation /
//!   denied, with a risk level and human-readable reasons.
//!
//! Risk classification for exec decisions is provided by
//! [`crate::capability::shell::classify_command`] — no duplication.

pub mod broker;
pub mod fs_gate;
pub mod policy;
pub mod shell_gate;

pub use broker::{Actor, PermissionBroker, PermissionDecision};
pub use policy::SandboxPolicy;
