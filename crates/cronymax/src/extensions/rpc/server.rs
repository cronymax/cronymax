//! Per-extension RPC dispatcher.
//!
//! Phase 2 implements this (`P2-T04`). Listens on the host's Unix socket /
//! Named Pipe, decodes [`super::codec`] frames, and routes by method into
//! the [`crate::extensions::api`] handlers.

use crate::extensions::error::ExtensionResult;

#[derive(Debug, Default)]
pub struct RpcServer;

impl RpcServer {
    pub fn new() -> Self {
        Self
    }

    pub async fn serve(&self) -> ExtensionResult<()> {
        Err(crate::extensions::ExtensionError::Rpc(
            "rpc serve loop is implemented in Phase 2 (P2-T04)".into(),
        ))
    }
}
