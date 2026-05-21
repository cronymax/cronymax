//! L2 extension-point registry — the *only* place per-EP wiring lives.
//!
//! Hard invariant (spec §1, plan §2.2): every L2 EP goes through this
//! module. There is no per-EP handler file; each new EP adds:
//!
//! 1. A field on [`crate::extensions::manifest::Contributes`] (already
//!    done for the six v1 points)
//! 2. One [`Self::ingest`] match arm
//! 3. (Optionally) a typed runtime registry next to
//!    [`crate::extensions::api::agents::AgentProviderRegistry`] when the
//!    EP needs RPC plumbing (commands / agents / renderers / sidebar)
//!
//! [`ContributionRegistry`] itself is just a JSON snapshot keyed by EP id;
//! callers downcast the `value` by EP. The platform consumers that wire
//! the live RPC connection (chat panel reading agent providers, command
//! palette listing commands, etc.) read the typed registries; this
//! registry is the source of truth for "what does the manifest say?" —
//! useful for the settings panel and for the v1 acceptance demos.
//!
//! EP id strings match `cep-idl/v1/manifest.ts` and the `#[serde(rename)]`
//! attributes on [`crate::extensions::manifest::Contributes`].

use std::collections::HashMap;

use crate::extensions::manifest::Manifest;

/// All v1 L2 extension-point ids. Kept as constants so consumers don't
/// stringly-type the names at every call site.
pub mod ep {
    pub const COMMAND: &str = "cronymax.command";
    pub const CONFIG_SCHEMA: &str = "cronymax.config.schema";
    pub const CONFIG_PAGE: &str = "cronymax.config.page";
    pub const AGENTS_PROVIDER: &str = "cronymax.agents.provider";
    pub const CONTENT_RENDERER: &str = "cronymax.content.renderer";
    pub const UI_SIDEBAR_VIEW: &str = "cronymax.ui.sidebar.view";
}

/// One typed entry inside the registry. The `value` is the raw JSON of
/// whatever the manifest declared at this EP; per-EP consumers decide
/// how to interpret it.
#[derive(Clone, Debug)]
pub struct ContributionEntry {
    pub ext_id: String,
    pub ep_id: String,
    pub value: serde_json::Value,
}

/// Every contribution registered at runtime, indexed by EP id and then
/// by owning extension id.
///
/// Within one `(ep_id, ext_id)` pair the value is either a single object
/// (for EPs that take a single object, like `cronymax.config.schema`) or
/// a JSON array of objects (for EPs that take a list, like
/// `cronymax.command`). Consumers can branch on `value.is_array()`.
#[derive(Debug, Default)]
pub struct ContributionRegistry {
    entries: HashMap<String, HashMap<String, ContributionEntry>>,
}

impl ContributionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reflect every `contributes.*` field of `manifest` into the
    /// registry. Overwrites any prior entries for this extension at
    /// the same EP (re-activation is idempotent).
    ///
    /// Returns the number of EP slots populated for this extension (0–6).
    /// Useful for tracing.
    pub fn ingest(&mut self, manifest: &Manifest) -> usize {
        let ext_id = &manifest.id;
        let c = &manifest.contributes;
        let mut populated = 0;

        if !c.commands.is_empty() {
            let v = serde_json::to_value(&c.commands).unwrap_or_else(|_| serde_json::json!([]));
            self.insert(ep::COMMAND, ext_id, v);
            populated += 1;
        }
        if let Some(schema) = &c.config_schema {
            self.insert(ep::CONFIG_SCHEMA, ext_id, schema.clone());
            populated += 1;
        }
        if !c.config_pages.is_empty() {
            let v = serde_json::to_value(&c.config_pages).unwrap_or_else(|_| serde_json::json!([]));
            self.insert(ep::CONFIG_PAGE, ext_id, v);
            populated += 1;
        }
        if !c.agent_providers.is_empty() {
            let v =
                serde_json::to_value(&c.agent_providers).unwrap_or_else(|_| serde_json::json!([]));
            self.insert(ep::AGENTS_PROVIDER, ext_id, v);
            populated += 1;
        }
        if !c.content_renderers.is_empty() {
            let v = serde_json::to_value(&c.content_renderers)
                .unwrap_or_else(|_| serde_json::json!([]));
            self.insert(ep::CONTENT_RENDERER, ext_id, v);
            populated += 1;
        }
        if !c.sidebar_views.is_empty() {
            let v =
                serde_json::to_value(&c.sidebar_views).unwrap_or_else(|_| serde_json::json!([]));
            self.insert(ep::UI_SIDEBAR_VIEW, ext_id, v);
            populated += 1;
        }
        populated
    }

    /// Drop every contribution owned by `ext_id` across all EPs.
    /// Returns the number of EP slots emptied. Called on deactivate.
    pub fn remove_extension(&mut self, ext_id: &str) -> usize {
        let mut removed = 0;
        for ep_map in self.entries.values_mut() {
            if ep_map.remove(ext_id).is_some() {
                removed += 1;
            }
        }
        // Drop now-empty EP maps so `iter`/`ep_ids` don't return phantoms.
        self.entries.retain(|_, m| !m.is_empty());
        removed
    }

    /// Iterate contributions for one EP. Returns an empty iterator if no
    /// extension contributed. Order is unspecified.
    pub fn for_ep<'a>(&'a self, ep_id: &str) -> impl Iterator<Item = &'a ContributionEntry> + 'a {
        self.entries.get(ep_id).into_iter().flat_map(|m| m.values())
    }

    /// Look up one extension's contribution at a specific EP.
    pub fn get(&self, ep_id: &str, ext_id: &str) -> Option<&ContributionEntry> {
        self.entries.get(ep_id).and_then(|m| m.get(ext_id))
    }

    /// All EP ids with at least one contribution, sorted.
    pub fn ep_ids(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.entries.keys().map(|s| s.as_str()).collect();
        v.sort_unstable();
        v
    }

    /// Total contribution count across all EPs. Each `(ep, ext)` pair
    /// counts as one even if the underlying array has multiple commands.
    pub fn len(&self) -> usize {
        self.entries.values().map(|m| m.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn insert(&mut self, ep_id: &str, ext_id: &str, value: serde_json::Value) {
        self.entries.entry(ep_id.to_string()).or_default().insert(
            ext_id.to_string(),
            ContributionEntry {
                ext_id: ext_id.to_string(),
                ep_id: ep_id.to_string(),
                value,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::manifest::Manifest;

    fn manifest_with_all_six() -> Manifest {
        let raw = r#"{
            "id": "alice.x",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": [],
            "contributes": {
                "cronymax.command": [
                    { "id": "alice.x.hi", "title": "Hi" }
                ],
                "cronymax.config.schema": {
                    "title": "Alice",
                    "properties": { "x": { "type": "string" } }
                },
                "cronymax.config.page": [
                    { "id": "p.main", "title": "Main", "entry": "./s.html" }
                ],
                "cronymax.agents.provider": [
                    { "id": "alice.x.gpt", "label": "Alice GPT" }
                ],
                "cronymax.content.renderer": [
                    { "id": "r1", "mimeTypes": ["text/x-foo"], "entry": "./r.html" }
                ],
                "cronymax.ui.sidebar.view": [
                    { "id": "v1", "title": "Main", "entry": "./v.html" }
                ]
            }
        }"#;
        Manifest::from_json(raw).unwrap()
    }

    fn manifest_with_only_commands(ext_id: &str, publisher: &str) -> Manifest {
        let raw = format!(
            r#"{{
                "id": "{ext_id}",
                "name": "X",
                "version": "0.1.0",
                "publisher": "{publisher}",
                "engines": {{ "cronymax": "^1.0" }},
                "main": "./m.js",
                "activationEvents": [],
                "contributes": {{
                    "cronymax.command": [
                        {{ "id": "{ext_id}.hello", "title": "Hello" }}
                    ]
                }}
            }}"#
        );
        Manifest::from_json(&raw).unwrap()
    }

    #[test]
    fn empty_manifest_ingests_nothing() {
        let mut reg = ContributionRegistry::new();
        let m = Manifest::default();
        let n = reg.ingest(&m);
        assert_eq!(n, 0);
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());
    }

    #[test]
    fn ingest_all_six_eps() {
        let mut reg = ContributionRegistry::new();
        let m = manifest_with_all_six();
        let n = reg.ingest(&m);
        assert_eq!(n, 6, "all six EP slots populated");
        assert_eq!(reg.len(), 6);
        let mut eps = reg.ep_ids();
        eps.sort_unstable();
        assert_eq!(
            eps,
            vec![
                ep::AGENTS_PROVIDER,
                ep::COMMAND,
                ep::CONFIG_PAGE,
                ep::CONFIG_SCHEMA,
                ep::CONTENT_RENDERER,
                ep::UI_SIDEBAR_VIEW,
            ]
        );
    }

    #[test]
    fn ingest_command_array_is_preserved() {
        let mut reg = ContributionRegistry::new();
        let m = manifest_with_only_commands("alice.x", "alice");
        reg.ingest(&m);
        let entry = reg.get(ep::COMMAND, "alice.x").unwrap();
        assert!(entry.value.is_array(), "commands serialize as a JSON array");
        let arr = entry.value.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "alice.x.hello");
        assert_eq!(arr[0]["title"], "Hello");
    }

    #[test]
    fn config_schema_is_stored_as_object_not_array() {
        let mut reg = ContributionRegistry::new();
        let m = manifest_with_all_six();
        reg.ingest(&m);
        let entry = reg.get(ep::CONFIG_SCHEMA, "alice.x").unwrap();
        assert!(
            entry.value.is_object(),
            "config schema is a single object, not an array"
        );
        assert_eq!(entry.value["title"], "Alice");
    }

    #[test]
    fn multiple_extensions_coexist_at_same_ep() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_only_commands("alice.x", "alice"));
        reg.ingest(&manifest_with_only_commands("bob.y", "bob"));

        let mut seen: Vec<String> = reg.for_ep(ep::COMMAND).map(|e| e.ext_id.clone()).collect();
        seen.sort_unstable();
        assert_eq!(seen, vec!["alice.x", "bob.y"]);
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn same_extension_ingested_twice_overwrites_not_duplicates() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_only_commands("alice.x", "alice"));
        reg.ingest(&manifest_with_only_commands("alice.x", "alice"));
        // One entry, not two.
        assert_eq!(reg.for_ep(ep::COMMAND).count(), 1);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn remove_extension_clears_every_ep_slot_it_owned() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_all_six());
        reg.ingest(&manifest_with_only_commands("bob.y", "bob"));

        let removed = reg.remove_extension("alice.x");
        assert_eq!(
            removed, 6,
            "alice.x contributed to all six EPs; all should be cleared"
        );
        // bob.y's command contribution survives.
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.for_ep(ep::COMMAND).count(), 1);
        assert!(reg.get(ep::AGENTS_PROVIDER, "alice.x").is_none());
    }

    #[test]
    fn remove_unknown_extension_is_a_zero_return_noop() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_only_commands("alice.x", "alice"));
        assert_eq!(reg.remove_extension("ghost"), 0);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn for_ep_on_unknown_ep_is_empty() {
        let reg = ContributionRegistry::new();
        assert_eq!(reg.for_ep("cronymax.totally.fake").count(), 0);
    }

    #[test]
    fn ep_constants_match_manifest_serde_renames() {
        // Defence in depth: if anyone bumps the dotted JSON key on
        // `Contributes` without also updating the EP constant, the
        // entry would silently land at the wrong EP id. Pin the round
        // trip here.
        let m = manifest_with_all_six();
        let mut reg = ContributionRegistry::new();
        reg.ingest(&m);
        for ep in [
            ep::COMMAND,
            ep::CONFIG_SCHEMA,
            ep::CONFIG_PAGE,
            ep::AGENTS_PROVIDER,
            ep::CONTENT_RENDERER,
            ep::UI_SIDEBAR_VIEW,
        ] {
            assert!(
                reg.get(ep, "alice.x").is_some(),
                "no contribution for EP `{ep}` — the EP id constant likely drifted from the serde(rename) on Contributes",
            );
        }
    }
}
