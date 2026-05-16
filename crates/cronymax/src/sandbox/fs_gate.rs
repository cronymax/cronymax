//! [`PolicyFilesystem`]: a thin wrapper around a [`FilesystemCapability`]
//! implementation that enforces [`SandboxPolicy`] read/write checks via
//! [`PermissionBroker`] before delegating each I/O operation.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use super::broker::{Actor, PermissionBroker};
use super::policy::SandboxPolicy;
use crate::capability::filesystem::{FilesystemCapability, ReadFileResult, StrReplaceResult};

/// Filesystem capability wrapper that checks [`SandboxPolicy`] before
/// delegating to the inner implementation.
#[derive(Debug)]
pub struct PolicyFilesystem<F> {
    inner: F,
    broker: PermissionBroker,
    policy: Arc<SandboxPolicy>,
}

impl<F: FilesystemCapability> PolicyFilesystem<F> {
    pub fn new(inner: F, broker: PermissionBroker, policy: Arc<SandboxPolicy>) -> Self {
        Self {
            inner,
            broker,
            policy,
        }
    }
}

#[async_trait]
impl<F: FilesystemCapability> FilesystemCapability for PolicyFilesystem<F> {
    async fn read_file(
        &self,
        path: &Path,
        offset: Option<u64>,
        max_bytes: Option<u64>,
    ) -> anyhow::Result<ReadFileResult> {
        let decision = self.broker.check_read(Actor::Agent, path, &self.policy);
        if !decision.allowed {
            anyhow::bail!(
                "sandbox policy denied read access to '{}': {}",
                path.display(),
                decision.reason
            );
        }
        self.inner.read_file(path, offset, max_bytes).await
    }

    async fn write_file(
        &self,
        path: &Path,
        content: &str,
        create_dirs: bool,
    ) -> anyhow::Result<()> {
        let decision = self.broker.check_write(Actor::Agent, path, &self.policy);
        if !decision.allowed {
            anyhow::bail!(
                "sandbox policy denied write access to '{}': {}",
                path.display(),
                decision.reason
            );
        }
        self.inner.write_file(path, content, create_dirs).await
    }

    async fn list_dir(&self, path: &Path) -> anyhow::Result<Vec<String>> {
        let decision = self.broker.check_read(Actor::Agent, path, &self.policy);
        if !decision.allowed {
            anyhow::bail!(
                "sandbox policy denied read access to '{}': {}",
                path.display(),
                decision.reason
            );
        }
        self.inner.list_dir(path).await
    }

    async fn read_secret(&self, name: &str) -> anyhow::Result<String> {
        // Secret access is not path-based; always allow (secrets are managed
        // by the host and are not subject to filesystem sandbox rules).
        self.inner.read_secret(name).await
    }

    async fn str_replace(
        &self,
        path: &Path,
        old_str: &str,
        new_str: &str,
    ) -> anyhow::Result<StrReplaceResult> {
        let decision = self.broker.check_write(Actor::Agent, path, &self.policy);
        if !decision.allowed {
            anyhow::bail!(
                "sandbox policy denied write access to '{}': {}",
                path.display(),
                decision.reason
            );
        }
        self.inner.str_replace(path, old_str, new_str).await
    }
}
