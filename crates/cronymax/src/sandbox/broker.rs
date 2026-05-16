//! [`PermissionBroker`]: evaluates exec / read / write decisions against a
//! [`SandboxPolicy`]. Mirrors `app/sandbox/PermissionBroker`.
//!
//! The broker is stateless; all policy state lives in [`SandboxPolicy`].
//! Risk classification for exec decisions is delegated to
//! [`crate::capability::shell::classify_command`].

use std::path::Path;

use crate::capability::shell::{classify_command, RiskLevel};

use super::policy::SandboxPolicy;

// ── Actor ─────────────────────────────────────────────────────────────────────

/// The principal requesting an operation. Mirrors `common/types.h Actor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    User,
    Agent,
}

// ── PermissionDecision ────────────────────────────────────────────────────────

/// Outcome of a permission check.
#[derive(Clone, Debug)]
pub struct PermissionDecision {
    /// Whether the operation is permitted at all.
    pub allowed: bool,
    /// The operation is permitted but requires explicit human confirmation
    /// before execution (e.g. high-risk shell commands).
    pub requires_confirmation: bool,
    /// Assessed risk of the operation.
    pub risk: RiskLevel,
    /// Human-readable summary (empty when unconditionally allowed).
    pub reason: String,
    /// Individual risk signals that contributed to the decision.
    pub risk_reasons: Vec<String>,
}

impl PermissionDecision {
    fn allow() -> Self {
        Self {
            allowed: true,
            requires_confirmation: false,
            risk: RiskLevel::Low,
            reason: String::new(),
            risk_reasons: vec![],
        }
    }

    fn confirm(risk: RiskLevel, reason: impl Into<String>, risk_reasons: Vec<String>) -> Self {
        Self {
            allowed: true,
            requires_confirmation: true,
            risk,
            reason: reason.into(),
            risk_reasons,
        }
    }

    fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            requires_confirmation: false,
            risk: RiskLevel::High,
            reason: reason.into(),
            risk_reasons: vec![],
        }
    }
}

// ── PermissionBroker ──────────────────────────────────────────────────────────

/// Evaluates whether a given operation is allowed under a [`SandboxPolicy`].
///
/// The broker is stateless; instantiate it once and reuse.
#[derive(Clone, Debug, Default)]
pub struct PermissionBroker;

impl PermissionBroker {
    pub fn new() -> Self {
        Self
    }

    /// Check whether `actor` may execute `command` under `policy`.
    ///
    /// High-risk commands (sudo, rm -rf, piped downloads, …) require
    /// confirmation even if the policy would otherwise allow execution.
    /// Network commands require confirmation when the policy disables
    /// network access.
    pub fn check_exec(
        &self,
        _actor: Actor,
        command: &str,
        policy: &SandboxPolicy,
    ) -> PermissionDecision {
        let risk = classify_command(command);
        let mut reasons: Vec<String> = Vec::new();

        if !policy.allow_network() && is_network_command(command) {
            reasons.push("network access is disabled by the active sandbox policy".into());
        }

        match risk {
            RiskLevel::High => {
                reasons.push(format!("high-risk command pattern: `{command}`"));
                PermissionDecision::confirm(
                    risk,
                    "command requires explicit human approval",
                    reasons,
                )
            }
            RiskLevel::Medium if !reasons.is_empty() => {
                PermissionDecision::confirm(risk, "elevated-risk command", reasons)
            }
            _ if !reasons.is_empty() => {
                PermissionDecision::confirm(RiskLevel::Medium, "policy constraint", reasons)
            }
            _ => PermissionDecision::allow(),
        }
    }

    /// Check whether `actor` may read `path` under `policy`.
    pub fn check_read(
        &self,
        _actor: Actor,
        path: &Path,
        policy: &SandboxPolicy,
    ) -> PermissionDecision {
        if policy.can_read(path) {
            PermissionDecision::allow()
        } else {
            PermissionDecision::deny(format!(
                "path '{}' is outside the readable scope",
                path.display()
            ))
        }
    }

    /// Check whether `actor` may write `path` under `policy`.
    pub fn check_write(
        &self,
        _actor: Actor,
        path: &Path,
        policy: &SandboxPolicy,
    ) -> PermissionDecision {
        if policy.can_write(path) {
            PermissionDecision::allow()
        } else {
            PermissionDecision::deny(format!(
                "path '{}' is outside the writable scope",
                path.display()
            ))
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` if `command` contains a network-accessing tool invocation.
fn is_network_command(command: &str) -> bool {
    let lower = command.to_lowercase();
    ["curl ", "wget ", "ssh ", "rsync ", "nc ", "netcat ", "ftp "]
        .iter()
        .any(|&tok| lower.contains(tok))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> SandboxPolicy {
        SandboxPolicy::default_for_workspace("/ws")
    }

    #[test]
    fn safe_command_allowed() {
        let b = PermissionBroker::new();
        let d = b.check_exec(Actor::Agent, "ls -la", &default_policy());
        assert!(d.allowed && !d.requires_confirmation);
    }

    #[test]
    fn sudo_requires_confirmation() {
        let b = PermissionBroker::new();
        let d = b.check_exec(Actor::Agent, "sudo apt update", &default_policy());
        assert!(d.allowed && d.requires_confirmation);
        assert_eq!(d.risk, RiskLevel::High);
    }

    #[test]
    fn curl_blocked_when_no_network() {
        let b = PermissionBroker::new();
        // No network policy + medium-risk curl → confirm
        let d = b.check_exec(
            Actor::Agent,
            "curl -o file.txt https://example.com/data.txt",
            &default_policy(),
        );
        assert!(d.allowed && d.requires_confirmation);
    }

    #[test]
    fn curl_allowed_when_network_enabled() {
        let b = PermissionBroker::new();
        let mut policy = default_policy();
        policy.set_allow_network(true);
        let d = b.check_exec(
            Actor::Agent,
            "curl -o file.txt https://example.com/data.txt",
            &policy,
        );
        // medium-risk + network allowed → no confirmation needed
        assert!(d.allowed && !d.requires_confirmation);
    }

    #[test]
    fn read_in_scope_allowed() {
        let b = PermissionBroker::new();
        let d = b.check_read(Actor::Agent, Path::new("/ws/src/lib.rs"), &default_policy());
        assert!(d.allowed);
    }

    #[test]
    fn read_outside_scope_denied() {
        let b = PermissionBroker::new();
        let d = b.check_read(Actor::Agent, Path::new("/etc/passwd"), &default_policy());
        assert!(!d.allowed);
    }

    #[test]
    fn write_outside_scope_denied() {
        let b = PermissionBroker::new();
        let d = b.check_write(
            Actor::User,
            Path::new("/usr/local/bin/evil"),
            &default_policy(),
        );
        assert!(!d.allowed);
    }
}
