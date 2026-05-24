//! Resolve paths to the bundled extension-host assets at runtime.
//!
//! Spawn time needs two artifacts shipped with the cronymax binary:
//!
//! * the Node 26 binary  → `<bundled>/node/bin/node`
//! * the bootstrap script → `<bundled>/extension-host-bootstrap.js`
//!
//! Resolution order (first hit wins):
//!
//! 1. `CRONYMAX_BUNDLED_DIR` env override — used by packaging and tests.
//! 2. Walk up from `std::env::current_exe()`, checking each ancestor for
//!    either a sibling `bundled/` (future Resources layout) or the
//!    in-repo `crates/cronymax/bundled/` layout. This covers
//!    `cargo run -p crony` and the .app bundle once packaging lands.
//! 3. `CARGO_MANIFEST_DIR/bundled/` — only present when built via cargo
//!    (covers integration tests that exercise `RuntimeServices::new`).
//!
//! Returns `None` when no candidate matches; the caller logs a warning
//! and falls back to no startup activation, keeping the rest of the
//! runtime functional.

use std::path::{Path, PathBuf};

const BOOTSTRAP_FILENAME: &str = "extension-host-bootstrap.js";

/// Path to the bundled root that contains `extension-host-bootstrap.js`
/// and the `node/` sub-tree.
pub fn default_bundled_dir() -> Option<PathBuf> {
    if let Some(val) = std::env::var_os("CRONYMAX_BUNDLED_DIR") {
        let candidate = PathBuf::from(val);
        if candidate.join(BOOTSTRAP_FILENAME).is_file() {
            return Some(candidate);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(start) = exe.parent() {
            if let Some(found) = walk_up_for_bundled(start) {
                return Some(found);
            }
        }
    }

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let candidate = PathBuf::from(manifest_dir).join("bundled");
    if candidate.join(BOOTSTRAP_FILENAME).is_file() {
        return Some(candidate);
    }

    None
}

/// Absolute path to the bundled Node 26 binary, or `None` when the
/// bundle is unavailable.
pub fn default_bundled_node() -> Option<PathBuf> {
    default_bundled_dir().map(|d| d.join("node").join("bin").join("node"))
}

/// Absolute path to `extension-host-bootstrap.js`, or `None` when the
/// bundle is unavailable.
pub fn default_bundled_bootstrap() -> Option<PathBuf> {
    default_bundled_dir().map(|d| d.join(BOOTSTRAP_FILENAME))
}

fn walk_up_for_bundled(start: &Path) -> Option<PathBuf> {
    let mut cursor: Option<&Path> = Some(start);
    while let Some(dir) = cursor {
        let direct = dir.join("bundled");
        if direct.join(BOOTSTRAP_FILENAME).is_file() {
            return Some(direct);
        }
        let nested = dir.join("crates").join("cronymax").join("bundled");
        if nested.join(BOOTSTRAP_FILENAME).is_file() {
            return Some(nested);
        }
        cursor = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn env_override_takes_priority() {
        let td = TempDir::new().unwrap();
        std::fs::write(td.path().join(BOOTSTRAP_FILENAME), "// hi\n").unwrap();
        let prev = std::env::var_os("CRONYMAX_BUNDLED_DIR");
        std::env::set_var("CRONYMAX_BUNDLED_DIR", td.path());
        let resolved = default_bundled_dir().unwrap();
        assert_eq!(resolved, td.path());
        match prev {
            Some(v) => std::env::set_var("CRONYMAX_BUNDLED_DIR", v),
            None => std::env::remove_var("CRONYMAX_BUNDLED_DIR"),
        }
    }

    #[test]
    fn walk_up_finds_nested_layout() {
        let td = TempDir::new().unwrap();
        let bundled = td.path().join("crates/cronymax/bundled");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::write(bundled.join(BOOTSTRAP_FILENAME), "// hi\n").unwrap();
        // Pretend we ran from `<root>/target/debug/`.
        let deep = td.path().join("target/debug");
        std::fs::create_dir_all(&deep).unwrap();
        let found = walk_up_for_bundled(&deep).unwrap();
        assert_eq!(found, bundled);
    }

    #[test]
    fn walk_up_finds_sibling_layout() {
        let td = TempDir::new().unwrap();
        let bundled = td.path().join("Resources/bundled");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::write(bundled.join(BOOTSTRAP_FILENAME), "// hi\n").unwrap();
        // Walking up from a sibling dir of `bundled/`.
        let deep = td.path().join("Resources/macos");
        std::fs::create_dir_all(&deep).unwrap();
        let found = walk_up_for_bundled(&deep).unwrap();
        assert_eq!(found, bundled);
    }

    #[test]
    fn walk_up_returns_none_when_absent() {
        let td = TempDir::new().unwrap();
        assert!(walk_up_for_bundled(td.path()).is_none());
    }
}
