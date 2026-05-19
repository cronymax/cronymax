//! `Manifest` → Node 26 `--allow-*` flags translation.
//!
//! **Phase 2 implements `build_node_flags`.** This is the *only* enforcement
//! layer for fs / network / process / workers / addons / ffi / inspector.
//! The Rust side never wraps `fs` / `child_process` / `fetch`; Node's
//! Permission Model is the security boundary.
//!
//! Spec reference: `docs/extensions/spec-v0.3.md` §6.1.

use std::path::Path;

use super::error::ExtensionResult;
use super::manifest::Manifest;

/// Build the argv tail (`--permission --allow-*=...`) Rust hands to the Node
/// subprocess. The function MUST:
///
/// 1. start with `--permission`
/// 2. always grant `--allow-fs-read` / `--allow-fs-write` to the
///    per-extension storage dir (so `ctx.storageUri` always works)
/// 3. canonicalize every path (macOS `/tmp` → `/private/tmp` etc.)
/// 4. translate `capabilities.network.allow` 1:1 to `--allow-net=<host>`
/// 5. translate `capabilities.process: true` to `--allow-child-process`
/// 6. NEVER emit `--allow-ffi`, `--allow-inspector`, or `--allow-worker`
///    unless the manifest opts in (off by default).
///
/// Phase 2 task: `P2-T02`.
pub fn build_node_flags(
    _manifest: &Manifest,
    _workspace: &Path,
    _ext_dir: &Path,
    _storage_dir: &Path,
) -> ExtensionResult<Vec<String>> {
    Ok(vec!["--permission".to_string()])
}
