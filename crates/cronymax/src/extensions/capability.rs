//! Spawn-time context for the Node 26 host.
//!
//! **v1 alpha dropped the Node permission model.** Extensions run with
//! full Node API access; manifests do **not** enforce per-capability
//! `--allow-*` flags. The trust boundary moved from "code-level OS gates"
//! to "author-level install-time trust" (see spec §6 revision and
//! `docs/extensions/permission-removal.md` for the rationale).
//!
//! What this module still does:
//!
//! * Carries the canonical paths the host hands to `bootstrap.js` via env
//!   vars ([`ExpansionCtx`]) — workspace folders, extension dirs, storage
//!   dirs, home, tmp, cronymax config root
//! * Produces the spawn-time argv tail ([`build_node_flags`]) — currently
//!   only `--no-warnings` to silence Node experimental noise; future
//!   platform-side OS sandbox (sandbox-exec / bubblewrap) wraps the
//!   process *outside* Node and is independent of this module
//!
//! All previous logic for translating `manifest.capabilities.fs/network/
//! process/...` into `--allow-fs-* / --allow-net / --allow-child-process`
//! was deleted; the manifest no longer carries those fields.

use std::path::PathBuf;

use super::error::ExtensionResult;
use super::manifest::Manifest;

/// Per-spawn context for path information the host wants to pass through
/// to `bootstrap.js` via environment variables. Not a permission boundary.
#[derive(Clone, Debug)]
pub struct ExpansionCtx {
    /// All currently open workspace roots. Empty when no workspace is open.
    pub workspaces: Vec<PathBuf>,
    /// Extension install dir, e.g. `~/.cronymax/extensions/<id>/`.
    pub ext_dir: PathBuf,
    /// Per-extension private storage, e.g.
    /// `~/.cronymax/extensions/<id>/storage/`.
    pub ext_storage: PathBuf,
    /// Per-extension cross-workspace storage, e.g.
    /// `~/.cronymax/global-state/<id>/`.
    pub ext_global_storage: PathBuf,
    /// User home directory (`$HOME` / `%USERPROFILE%`).
    pub home: PathBuf,
    /// `os.tmpdir()` output.
    pub tmp: PathBuf,
    /// `~/.cronymax/` — cronymax-wide config root.
    pub cronymax_config: PathBuf,
}

/// Build the argv tail the Rust host hands to the Node 26 subprocess.
///
/// v1 alpha emits only `--no-warnings` — there is no permission gate.
/// `manifest` and `ctx` are unused but kept in the signature for forward
/// compatibility (an opt-in OS sandbox in v1.x may consult them).
pub fn build_node_flags(_manifest: &Manifest, _ctx: &ExpansionCtx) -> ExtensionResult<Vec<String>> {
    Ok(vec!["--no-warnings".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ctx(home: &Path, tmp: &Path) -> ExpansionCtx {
        ExpansionCtx {
            workspaces: Vec::new(),
            ext_dir: home.join(".cronymax/extensions/alice.x"),
            ext_storage: home.join(".cronymax/extensions/alice.x/storage"),
            ext_global_storage: home.join(".cronymax/global-state/alice.x"),
            home: home.to_path_buf(),
            tmp: tmp.to_path_buf(),
            cronymax_config: home.join(".cronymax"),
        }
    }

    fn manifest() -> Manifest {
        let raw = r#"{
            "id": "alice.x",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": []
        }"#;
        Manifest::from_json(raw).unwrap()
    }

    #[test]
    fn emits_only_no_warnings() {
        let m = manifest();
        let c = ctx(Path::new("/tmp/home"), Path::new("/tmp"));
        let flags = build_node_flags(&m, &c).unwrap();
        assert_eq!(flags, vec!["--no-warnings".to_string()]);
    }

    #[test]
    fn does_not_emit_permission_flag() {
        // Regression check: the v1 alpha decision was to drop --permission
        // entirely. Re-introducing it would be a load-bearing policy
        // change and must come with an explicit spec amendment.
        let m = manifest();
        let c = ctx(Path::new("/tmp/home"), Path::new("/tmp"));
        let flags = build_node_flags(&m, &c).unwrap();
        assert!(
            !flags.iter().any(|f| f == "--permission"),
            "--permission must not be emitted under v1 alpha; got {flags:?}",
        );
        assert!(
            !flags.iter().any(|f| f.starts_with("--allow-")),
            "no --allow-* flags should be emitted; got {flags:?}",
        );
    }
}
