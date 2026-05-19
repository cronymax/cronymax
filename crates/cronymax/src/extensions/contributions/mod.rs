//! L2 extension-point registry — the *only* place per-EP wiring lives.
//!
//! **Phase 4 implements this (`P4-T01`).** Hard invariant: every L2 EP goes
//! through this module. No per-EP handler file.
//!
//! Each EP is identified by a string id (`"cronymax.agents.provider"`,
//! `"cronymax.content.renderer"`, ...) and stored as a `Box<dyn Any>` keyed
//! by id, with platform-side consumers downcasting to a known trait when they
//! enumerate contributions.

use std::collections::HashMap;

use crate::extensions::manifest::Manifest;

/// Every contribution registered at runtime, indexed by EP id and then by
/// owning extension id. Phase 4 fills in the per-EP value shape.
#[derive(Debug, Default)]
pub struct ContributionRegistry {
    /// EP id → contributing extension id → opaque value. Phase 4 (`P4-T01`)
    /// populates this from `ContributionRegistry::ingest`.
    #[allow(dead_code)]
    entries: HashMap<String, HashMap<String, ContributionEntry>>,
}

#[derive(Debug)]
pub struct ContributionEntry {
    pub ext_id: String,
    pub ep_id: String,
    /// Raw JSON payload from the manifest. Phase 4 will define typed views.
    pub value: serde_json::Value,
}

impl ContributionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reflect contributions from a freshly-loaded manifest.
    pub fn ingest(&mut self, _manifest: &Manifest) {
        // Phase 4 (P4-T01) fills this in by iterating
        // `manifest.contributes.*` arrays and registering them under the
        // matching EP id.
    }

    /// Iterate contributions for one EP. Returns an empty iterator if no
    /// extension contributed.
    pub fn for_ep<'a>(&'a self, ep_id: &str) -> impl Iterator<Item = &'a ContributionEntry> + 'a {
        self.entries.get(ep_id).into_iter().flat_map(|m| m.values())
    }
}
