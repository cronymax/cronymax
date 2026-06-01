//! `workspace.rootUri` / `workspace.fs` RPC handlers.
//!
//! Phase 2 (`P2-T06`) implements. Note: `workspace.fs` here is a thin URI
//! facade — actual filesystem access is performed *inside* the Node host
//! via `fs/promises`, gated by `--allow-fs-*` flags. The Rust side only
//! resolves URIs and reports the workspace root.
//!
//! Mirrors `cep-idl/v1/workspace.ts`.

use std::path::PathBuf;

use crate::extensions::error::ExtensionResult;

pub async fn root_uri() -> ExtensionResult<Option<PathBuf>> {
    Ok(None)
}
