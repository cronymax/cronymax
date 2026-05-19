//! `cronymax-extension.json` manifest — schema + validation.
//!
//! **Phase 1 implements this.** Today it stubs the public types so other
//! modules can take `&Manifest` arguments without churn.
//!
//! The TS IDL lives next to this file at
//! [`cep-idl/v1/manifest.ts`](./cep-idl/v1/manifest.ts) and is the source of
//! truth. Field names match 1:1 (snake_case here, kebab/dotted at the JSON
//! layer is handled by `#[serde(rename = "...")]` when the real impl lands).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::error::{ExtensionError, ExtensionResult};

/// Parsed + validated `cronymax-extension.json`.
///
/// Fields mirror the TypeScript `Manifest` interface in
/// `cep-idl/v1/manifest.ts`. Optional fields keep `Option<T>` so growth in
/// future v1 patch releases is non-breaking.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// `<publisher>.<name>`. Must start with `publisher.`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,

    #[serde(default)]
    pub engines: Engines,

    /// Relative path inside the extension dir.
    pub main: String,

    pub description: Option<String>,
    pub icon: Option<String>,
    pub repository: Option<String>,
    pub license: Option<String>,
    pub keywords: Option<Vec<String>>,

    #[serde(rename = "activationEvents", default)]
    pub activation_events: Vec<String>,

    #[serde(default)]
    pub contributes: Contributes,

    #[serde(default)]
    pub capabilities: Capabilities,

    #[serde(rename = "extensionDependencies", default)]
    pub extension_dependencies: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Engines {
    /// SemVer range string, e.g. `^1.0`.
    pub cronymax: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Each entry declares one path scope; the platform expands variables
    /// (see [`FsCapability::path`]) and emits matching `--allow-fs-*` flags.
    /// Per-extension storage dirs are granted unconditionally and don't need
    /// to appear here.
    #[serde(default)]
    pub fs: Vec<FsCapability>,
    /// Informational only in v1 — Node 26's `--allow-net` is boolean; this
    /// list is surfaced in the install-time consent dialog.
    pub network: Option<NetworkCapability>,
    #[serde(default)]
    pub process: Option<bool>,
    #[serde(default)]
    pub workers: Option<bool>,
    #[serde(default, rename = "native_addons")]
    pub native_addons: Option<bool>,

    pub secrets: Option<SecretsCapability>,

    #[serde(default, rename = "events.subscribe")]
    pub events_subscribe: Vec<String>,
    #[serde(default, rename = "events.emit")]
    pub events_emit: Vec<String>,

    #[serde(default, rename = "ui-slots")]
    pub ui_slots: Vec<String>,
    #[serde(default, rename = "extension-points")]
    pub extension_points: Vec<String>,

    #[serde(default, rename = "auth.providers")]
    pub auth_providers: Vec<String>,
}

/// One filesystem grant. `path` MUST use one of the platform variables (see
/// [`crate::extensions::capability`]):
///
/// * `{WORKSPACE}`
/// * `{HOME}/<subpath>`
/// * `{EXT_DIR}`
/// * `{EXT_STORAGE}`
/// * `{EXT_GLOBAL_STORAGE}`
/// * `{TMP}/<subpath>`
/// * `{CRONYMAX_CONFIG}/<subpath>`
///
/// Absolute hard-coded paths are rejected at manifest validation. The
/// platform expands the variable at spawn time and canonicalises the result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FsCapability {
    pub path: String,
    /// `"r"` or `"rw"`.
    pub mode: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetworkCapability {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecretsCapability {
    /// Namespace prefix — must be inside the extension's publisher prefix.
    pub namespace: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Contributes {
    #[serde(default, rename = "cronymax.command")]
    pub commands: Vec<CommandContribution>,
    #[serde(default, rename = "cronymax.config.schema")]
    pub config_schema: Option<serde_json::Value>,
    #[serde(default, rename = "cronymax.config.page")]
    pub config_pages: Vec<ConfigPageContribution>,
    #[serde(default, rename = "cronymax.agents.provider")]
    pub agent_providers: Vec<AgentProviderContribution>,
    #[serde(default, rename = "cronymax.content.renderer")]
    pub content_renderers: Vec<ContentRendererContribution>,
    #[serde(default, rename = "cronymax.ui.sidebar.view")]
    pub sidebar_views: Vec<SidebarViewContribution>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandContribution {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigPageContribution {
    pub id: String,
    pub title: String,
    pub entry: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentProviderContribution {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub supports_models: Option<bool>,
    #[serde(default)]
    pub supports_modes: Option<bool>,
    #[serde(default)]
    pub supports_mcp: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentRendererContribution {
    pub id: String,
    #[serde(rename = "mimeTypes")]
    pub mime_types: Vec<String>,
    pub entry: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SidebarViewContribution {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub entry: String,
}

impl Manifest {
    /// Parse the on-disk JSON. **Phase 1 fills in validation.**
    pub fn from_json(_raw: &str) -> ExtensionResult<Self> {
        Err(ExtensionError::ManifestParse(
            "manifest parsing is implemented in Phase 1 (P1-T01)".into(),
        ))
    }

    /// Run all schema checks. **Phase 1 fills these in.**
    pub fn validate(&self, _ext_dir: &std::path::Path) -> ExtensionResult<()> {
        Err(ExtensionError::ManifestInvalid(
            "manifest validation is implemented in Phase 1 (P1-T02)".into(),
        ))
    }
}

/// What lives inside `~/.cronymax/extensions/<id>/` once installed.
#[derive(Clone, Debug)]
pub struct InstalledExtension {
    pub manifest: Manifest,
    pub ext_dir: PathBuf,
    /// Free-form extension-defined metadata, persisted across restarts.
    pub user_data: HashMap<String, serde_json::Value>,
}
