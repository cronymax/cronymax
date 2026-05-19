//! Installed-extension registry.
//!
//! **Phase 1 implements this.** Stubbed here so types are stable.
//!
//! The on-disk layout is:
//!
//! ```text
//! ~/.cronymax/extensions/
//! ├── registry.json                ← enable state, install timestamps
//! └── <publisher>.<name>/
//!     ├── cronymax-extension.json
//!     ├── dist/main.js
//!     └── ...
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::error::ExtensionResult;
use super::manifest::Manifest;

/// In-memory snapshot of all installed extensions plus their enabled state.
#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    /// Installation root, e.g. `~/.cronymax/extensions/`.
    root: PathBuf,
    /// `id` → entry. Stable across enable/disable. Phase 1 (`P1-T03`)
    /// populates this from `ExtensionRegistry::refresh`.
    #[allow(dead_code)]
    entries: HashMap<String, RegistryEntry>,
}

#[derive(Clone, Debug)]
pub struct RegistryEntry {
    pub manifest: Manifest,
    pub ext_dir: PathBuf,
    pub enabled: bool,
    /// Unix epoch seconds.
    pub installed_at: u64,
}

impl ExtensionRegistry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            entries: HashMap::new(),
        }
    }

    /// Re-scan the install dir from disk. **Phase 1 implements.**
    pub fn refresh(&mut self) -> ExtensionResult<()> {
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn get(&self, id: &str) -> Option<&RegistryEntry> {
        self.entries.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &RegistryEntry> {
        self.entries.values()
    }
}
