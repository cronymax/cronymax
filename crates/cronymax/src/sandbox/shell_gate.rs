//! [`PolicyShell`]: a thin wrapper around [`LocalShell`] that enforces a
//! [`SandboxPolicy`] via [`PermissionBroker::check_exec`] before delegating.

use std::sync::Arc;

use async_trait::async_trait;

use super::broker::{Actor, PermissionBroker};
use super::policy::SandboxPolicy;
use crate::capability::shell::{ExitStatus, ShellCapability, ShellRequest, ShellResult};

/// Shell capability wrapper that checks [`SandboxPolicy`] before delegating
/// to the inner implementation.
#[derive(Debug)]
pub struct PolicyShell<S> {
    inner: S,
    broker: PermissionBroker,
    policy: Arc<SandboxPolicy>,
}

impl<S: ShellCapability> PolicyShell<S> {
    pub fn new(inner: S, broker: PermissionBroker, policy: Arc<SandboxPolicy>) -> Self {
        Self {
            inner,
            broker,
            policy,
        }
    }
}

#[async_trait]
impl<S: ShellCapability> ShellCapability for PolicyShell<S> {
    async fn run(&self, request: ShellRequest) -> anyhow::Result<ShellResult> {
        let decision = self
            .broker
            .check_exec(Actor::Agent, &request.command, &self.policy);
        if !decision.allowed {
            return Ok(ShellResult {
                exit_status: ExitStatus::Code(126), // "command not permitted"
                stdout: String::new(),
                stderr: format!("sandbox policy denied execution: {}", decision.reason),
                elapsed_ms: 0,
            });
        }
        self.inner.run(request).await
    }

    fn max_output_bytes(&self) -> usize {
        self.inner.max_output_bytes()
    }
}
