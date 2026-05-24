//! Central contribution registry — every user-selectable thing (built-in
//! agent, workspace YAML agent, extension agent provider, command, content
//! renderer, sidebar view, …) is stored here as one
//! [`ContributionDescriptor`] keyed by `(kind, id)`.
//!
//! Three owner classes share the same table:
//!
//! * [`ContributionOwner::Platform`] — entries registered by Crony core
//!   itself (the built-in chat agent).
//! * [`ContributionOwner::Workspace`] — entries derived from files inside
//!   the active workspace (e.g. `<ws>/.cronymax/agents/*.agent.yaml`).
//! * [`ContributionOwner::Extension(ext_id)`] — entries declared in an
//!   extension manifest's `contributes.*` block.
//!
//! The chat / flow picker, the run dispatcher, and the settings UI all
//! talk to this one registry instead of stitching three separate sources
//! together. Picker dimensions (group/owner kind) and dispatcher dimensions
//! (which kind of agent to run) are derivable from the descriptor's
//! `kind` + `owner` pair.
//!
//! Wire contract: the JSON shape emitted to the chat panel (over the new
//! `contribution/list` and `contribution/enumerate` IPC variants) mirrors
//! `ContributionDescriptor` field-for-field. Field names match
//! `cep-idl/v1/contributions.ts` (kebab/camel handled at the serde layer).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::extensions::manifest::Manifest;

/// All v1 contribution-kind ids. Kept as constants so consumers don't
/// stringly-type the names at every call site.
///
/// The first six match the original v1 L2 extension-point ids 1:1; the
/// last three are new contribution kinds that don't come from a manifest
/// (platform / workspace sources).
pub mod kind {
    // Extension-declared (manifest `contributes.*`)
    pub const COMMAND: &str = "cronymax.command";
    pub const CONFIG_SCHEMA: &str = "cronymax.config.schema";
    pub const CONFIG_PAGE: &str = "cronymax.config.page";
    pub const AGENTS_PROVIDER: &str = "cronymax.agents.provider";
    pub const CONTENT_RENDERER: &str = "cronymax.content.renderer";
    pub const UI_SIDEBAR_VIEW: &str = "cronymax.ui.sidebar.view";

    // Platform / workspace registered at runtime, not from a manifest.
    pub const AGENTS_BUILTIN: &str = "cronymax.agents.builtin";
    pub const AGENTS_WORKSPACE: &str = "cronymax.agents.workspace";
}

/// Where this contribution came from. Mirrored on the wire as a tagged
/// union so the picker can group entries by source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContributionOwner {
    /// Registered by Crony core (e.g. the built-in chat agent sentinel).
    Platform,
    /// Derived from the active workspace's filesystem (YAML agents,
    /// project-local configs, …).
    Workspace,
    /// Declared by an extension manifest. `ext_id` is the manifest `id`.
    Extension {
        #[serde(rename = "extId")]
        ext_id: String,
    },
}

impl ContributionOwner {
    pub fn extension(ext_id: impl Into<String>) -> Self {
        Self::Extension {
            ext_id: ext_id.into(),
        }
    }

    pub fn ext_id(&self) -> Option<&str> {
        match self {
            Self::Extension { ext_id } => Some(ext_id.as_str()),
            _ => None,
        }
    }
}

/// One contribution. Stored in [`ContributionRegistry`] under `(kind, id)`.
///
/// `metadata` is the raw per-kind JSON payload — the caller that knows the
/// kind decodes it into the typed shape (e.g. `AgentProviderContribution`
/// for `cronymax.agents.provider`). For platform / workspace owners the
/// metadata mirrors whatever runtime info the source has (e.g. the agent
/// YAML body, the C++ Crony builtin descriptor).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContributionDescriptor {
    pub kind: String,
    pub id: String,
    pub owner: ContributionOwner,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Free-form per-kind payload. The shape depends on `kind`.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl ContributionDescriptor {
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        owner: ContributionOwner,
        label: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            owner,
            label: label.into(),
            description: None,
            icon: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// One enumerable child of a descriptor (e.g. a model under an agent
/// provider). Returned by `cronymax.agents.AgentProvider.enumerate()` and
/// the platform's workspace-agent introspection.
///
/// Items aren't stored in the registry — they're fetched on demand from
/// the descriptor's owner (extension RPC for provider items, in-memory
/// state for platform/workspace items). The registry only knows about
/// descriptors.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContributionItem {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Every contribution currently registered, indexed by `kind` then by
/// `id`. Within one `kind` the `id` must be globally unique — collisions
/// between an extension and the platform are rejected, but multiple
/// extensions can contribute to the same kind so long as their ids
/// differ.
#[derive(Debug, Default)]
pub struct ContributionRegistry {
    /// `kind` → `id` → descriptor.
    entries: HashMap<String, HashMap<String, ContributionDescriptor>>,
}

impl ContributionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert / overwrite one descriptor. Returns the previous descriptor
    /// at the same `(kind, id)` if one existed.
    pub fn add(&mut self, descriptor: ContributionDescriptor) -> Option<ContributionDescriptor> {
        self.entries
            .entry(descriptor.kind.clone())
            .or_default()
            .insert(descriptor.id.clone(), descriptor)
    }

    /// Remove one descriptor by `(kind, id)`.
    pub fn remove(&mut self, kind: &str, id: &str) -> Option<ContributionDescriptor> {
        let map = self.entries.get_mut(kind)?;
        let prev = map.remove(id);
        if map.is_empty() {
            self.entries.remove(kind);
        }
        prev
    }

    /// Reflect every `contributes.*` field of `manifest` into the registry
    /// as one descriptor per item. Overwrites any prior entries with the
    /// same `(kind, id)` (re-activation is idempotent).
    ///
    /// Returns the number of descriptors written.
    pub fn ingest(&mut self, manifest: &Manifest) -> usize {
        let ext_id = manifest.id.as_str();
        let owner = ContributionOwner::extension(ext_id);
        let c = &manifest.contributes;
        let mut written = 0usize;

        for cmd in &c.commands {
            let mut meta = serde_json::to_value(cmd).unwrap_or(serde_json::Value::Null);
            // Strip duplicated `id` / `title` from metadata for cleaner wire payloads.
            if let Some(obj) = meta.as_object_mut() {
                obj.remove("id");
            }
            let mut desc = ContributionDescriptor::new(
                kind::COMMAND,
                cmd.id.clone(),
                owner.clone(),
                cmd.title.clone(),
            )
            .with_metadata(meta);
            if let Some(icon) = &cmd.icon {
                desc = desc.with_icon(icon.clone());
            }
            if let Some(category) = &cmd.category {
                desc = desc.with_description(format!("Category: {category}"));
            }
            self.add(desc);
            written += 1;
        }

        if let Some(schema) = &c.config_schema {
            // Config schema is a single object per extension, addressed by
            // the owning ext id.
            let label = schema
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(ext_id)
                .to_string();
            let desc =
                ContributionDescriptor::new(kind::CONFIG_SCHEMA, ext_id, owner.clone(), label)
                    .with_metadata(schema.clone());
            self.add(desc);
            written += 1;
        }

        for page in &c.config_pages {
            let meta = serde_json::to_value(page).unwrap_or(serde_json::Value::Null);
            let desc = ContributionDescriptor::new(
                kind::CONFIG_PAGE,
                page.id.clone(),
                owner.clone(),
                page.title.clone(),
            )
            .with_metadata(meta);
            self.add(desc);
            written += 1;
        }

        for provider in &c.agent_providers {
            let meta = serde_json::to_value(provider).unwrap_or(serde_json::Value::Null);
            let mut desc = ContributionDescriptor::new(
                kind::AGENTS_PROVIDER,
                provider.id.clone(),
                owner.clone(),
                provider.label.clone(),
            )
            .with_metadata(meta);
            if let Some(icon) = &provider.icon {
                desc = desc.with_icon(icon.clone());
            }
            if let Some(d) = &provider.description {
                desc = desc.with_description(d.clone());
            }
            self.add(desc);
            written += 1;
        }

        for renderer in &c.content_renderers {
            let meta = serde_json::to_value(renderer).unwrap_or(serde_json::Value::Null);
            let label = renderer.mime_types.join(", ");
            let desc = ContributionDescriptor::new(
                kind::CONTENT_RENDERER,
                renderer.id.clone(),
                owner.clone(),
                label,
            )
            .with_metadata(meta);
            self.add(desc);
            written += 1;
        }

        for view in &c.sidebar_views {
            let meta = serde_json::to_value(view).unwrap_or(serde_json::Value::Null);
            let mut desc = ContributionDescriptor::new(
                kind::UI_SIDEBAR_VIEW,
                view.id.clone(),
                owner.clone(),
                view.title.clone(),
            )
            .with_metadata(meta);
            if let Some(icon) = &view.icon {
                desc = desc.with_icon(icon.clone());
            }
            self.add(desc);
            written += 1;
        }

        written
    }

    /// Drop every descriptor owned by `Extension(ext_id)`. Returns the
    /// number of descriptors removed. Called on extension deactivate.
    pub fn remove_extension(&mut self, ext_id: &str) -> usize {
        let mut removed = 0usize;
        for kind_map in self.entries.values_mut() {
            let before = kind_map.len();
            kind_map.retain(|_, d| d.owner.ext_id() != Some(ext_id));
            removed += before - kind_map.len();
        }
        self.entries.retain(|_, m| !m.is_empty());
        removed
    }

    /// Drop every descriptor whose owner is `Workspace`. Called when the
    /// active workspace changes or the workspace agent set is refreshed.
    pub fn remove_workspace(&mut self) -> usize {
        let mut removed = 0usize;
        for kind_map in self.entries.values_mut() {
            let before = kind_map.len();
            kind_map.retain(|_, d| !matches!(d.owner, ContributionOwner::Workspace));
            removed += before - kind_map.len();
        }
        self.entries.retain(|_, m| !m.is_empty());
        removed
    }

    /// Iterate descriptors of one kind. Order is unspecified.
    pub fn for_kind<'a>(
        &'a self,
        kind: &str,
    ) -> impl Iterator<Item = &'a ContributionDescriptor> + 'a {
        self.entries.get(kind).into_iter().flat_map(|m| m.values())
    }

    /// Look up one descriptor by `(kind, id)`.
    pub fn get(&self, kind: &str, id: &str) -> Option<&ContributionDescriptor> {
        self.entries.get(kind).and_then(|m| m.get(id))
    }

    /// Find any descriptor with the given `id`, scanning all kinds. Used
    /// by the run dispatcher when the caller passes a raw agent id without
    /// the kind tag.
    pub fn find_by_id(&self, id: &str) -> Option<&ContributionDescriptor> {
        for kind_map in self.entries.values() {
            if let Some(d) = kind_map.get(id) {
                return Some(d);
            }
        }
        None
    }

    /// All distinct kinds with at least one descriptor, sorted.
    pub fn kinds(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.entries.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// Snapshot every descriptor currently in the registry. Used by the
    /// `contribution/list` IPC variant.
    pub fn snapshot(&self) -> Vec<ContributionDescriptor> {
        let mut out = Vec::with_capacity(self.len());
        for kind_map in self.entries.values() {
            for d in kind_map.values() {
                out.push(d.clone());
            }
        }
        out
    }

    /// Total descriptor count across all kinds.
    pub fn len(&self) -> usize {
        self.entries.values().map(|m| m.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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

    fn manifest_with_two_commands(ext_id: &str, publisher: &str) -> Manifest {
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
                        {{ "id": "{ext_id}.hello", "title": "Hello" }},
                        {{ "id": "{ext_id}.bye", "title": "Bye" }}
                    ]
                }}
            }}"#
        );
        Manifest::from_json(&raw).unwrap()
    }

    #[test]
    fn empty_manifest_ingests_nothing() {
        let mut reg = ContributionRegistry::new();
        let n = reg.ingest(&Manifest::default());
        assert_eq!(n, 0);
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());
    }

    #[test]
    fn ingest_all_six_kinds_one_descriptor_each() {
        let mut reg = ContributionRegistry::new();
        let n = reg.ingest(&manifest_with_all_six());
        // 1 command + 1 schema + 1 page + 1 provider + 1 renderer + 1 view = 6
        assert_eq!(n, 6);
        assert_eq!(reg.len(), 6);
        assert!(reg.get(kind::COMMAND, "alice.x.hi").is_some());
        assert!(reg.get(kind::CONFIG_SCHEMA, "alice.x").is_some());
        assert!(reg.get(kind::CONFIG_PAGE, "p.main").is_some());
        assert!(reg.get(kind::AGENTS_PROVIDER, "alice.x.gpt").is_some());
        assert!(reg.get(kind::CONTENT_RENDERER, "r1").is_some());
        assert!(reg.get(kind::UI_SIDEBAR_VIEW, "v1").is_some());
    }

    #[test]
    fn ingest_multiple_commands_yields_one_descriptor_each() {
        let mut reg = ContributionRegistry::new();
        let n = reg.ingest(&manifest_with_two_commands("alice.x", "alice"));
        assert_eq!(n, 2);
        assert_eq!(reg.for_kind(kind::COMMAND).count(), 2);
        let hello = reg.get(kind::COMMAND, "alice.x.hello").unwrap();
        assert_eq!(hello.label, "Hello");
        assert_eq!(hello.owner, ContributionOwner::extension("alice.x"));
    }

    #[test]
    fn config_schema_descriptor_uses_ext_id_as_descriptor_id() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_all_six());
        let schema = reg.get(kind::CONFIG_SCHEMA, "alice.x").unwrap();
        assert!(schema.metadata.is_object());
        assert_eq!(schema.metadata["title"], "Alice");
        assert_eq!(schema.label, "Alice");
    }

    #[test]
    fn multiple_extensions_coexist_at_same_kind() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_two_commands("alice.x", "alice"));
        reg.ingest(&manifest_with_two_commands("bob.y", "bob"));
        assert_eq!(reg.for_kind(kind::COMMAND).count(), 4);
        // Lookups stay precise even across extensions.
        assert_eq!(
            reg.get(kind::COMMAND, "alice.x.hello").unwrap().owner,
            ContributionOwner::extension("alice.x")
        );
        assert_eq!(
            reg.get(kind::COMMAND, "bob.y.hello").unwrap().owner,
            ContributionOwner::extension("bob.y")
        );
    }

    #[test]
    fn same_extension_ingested_twice_overwrites_not_duplicates() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_two_commands("alice.x", "alice"));
        reg.ingest(&manifest_with_two_commands("alice.x", "alice"));
        assert_eq!(reg.for_kind(kind::COMMAND).count(), 2);
    }

    #[test]
    fn remove_extension_clears_every_kind_it_owned() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_all_six());
        reg.ingest(&manifest_with_two_commands("bob.y", "bob"));

        let removed = reg.remove_extension("alice.x");
        assert_eq!(removed, 6);
        // bob.y's two commands survive.
        assert_eq!(reg.for_kind(kind::COMMAND).count(), 2);
        assert!(reg.get(kind::AGENTS_PROVIDER, "alice.x.gpt").is_none());
        assert!(reg.get(kind::CONFIG_SCHEMA, "alice.x").is_none());
    }

    #[test]
    fn remove_unknown_extension_is_zero_noop() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_two_commands("alice.x", "alice"));
        assert_eq!(reg.remove_extension("ghost"), 0);
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn programmatic_add_and_remove_platform_descriptor() {
        let mut reg = ContributionRegistry::new();
        reg.add(
            ContributionDescriptor::new(
                kind::AGENTS_BUILTIN,
                "crony.builtin",
                ContributionOwner::Platform,
                "Crony",
            )
            .with_description("Built-in chat agent"),
        );
        assert_eq!(reg.len(), 1);
        let entry = reg.get(kind::AGENTS_BUILTIN, "crony.builtin").unwrap();
        assert_eq!(entry.owner, ContributionOwner::Platform);
        let removed = reg.remove(kind::AGENTS_BUILTIN, "crony.builtin");
        assert!(removed.is_some());
        assert!(reg.is_empty());
    }

    #[test]
    fn remove_workspace_only_drops_workspace_owned() {
        let mut reg = ContributionRegistry::new();
        reg.add(ContributionDescriptor::new(
            kind::AGENTS_WORKSPACE,
            "ws.dev",
            ContributionOwner::Workspace,
            "Dev",
        ));
        reg.add(ContributionDescriptor::new(
            kind::AGENTS_BUILTIN,
            "crony.builtin",
            ContributionOwner::Platform,
            "Crony",
        ));
        reg.ingest(&manifest_with_two_commands("alice.x", "alice"));

        let removed = reg.remove_workspace();
        assert_eq!(removed, 1);
        assert!(reg.get(kind::AGENTS_BUILTIN, "crony.builtin").is_some());
        assert_eq!(reg.for_kind(kind::COMMAND).count(), 2);
    }

    #[test]
    fn find_by_id_scans_all_kinds() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_all_six());
        let d = reg.find_by_id("alice.x.gpt").unwrap();
        assert_eq!(d.kind, kind::AGENTS_PROVIDER);
    }

    #[test]
    fn snapshot_returns_every_descriptor() {
        let mut reg = ContributionRegistry::new();
        reg.ingest(&manifest_with_all_six());
        let snap = reg.snapshot();
        assert_eq!(snap.len(), 6);
        let kinds: std::collections::BTreeSet<_> = snap.iter().map(|d| d.kind.clone()).collect();
        assert!(kinds.contains(kind::COMMAND));
        assert!(kinds.contains(kind::AGENTS_PROVIDER));
    }

    #[test]
    fn for_kind_on_unknown_kind_is_empty() {
        let reg = ContributionRegistry::new();
        assert_eq!(reg.for_kind("cronymax.totally.fake").count(), 0);
    }

    #[test]
    fn owner_serializes_as_tagged_union() {
        let p = ContributionOwner::Platform;
        let w = ContributionOwner::Workspace;
        let e = ContributionOwner::extension("alice.x");
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!({"type": "platform"})
        );
        assert_eq!(
            serde_json::to_value(&w).unwrap(),
            serde_json::json!({"type": "workspace"})
        );
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"type": "extension", "extId": "alice.x"})
        );
    }
}
