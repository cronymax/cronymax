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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// `<publisher>.<name>`. Must start with `publisher.`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,

    #[serde(default)]
    pub engines: Engines,

    /// Relative path to the Node-side entry point, inside the extension
    /// dir. **Optional** — extensions that ship only declarative
    /// contributions (e.g. a content renderer with no Node-side
    /// coordinator) may omit `main`; the platform recognises them and
    /// skips spawning a Node host. Mirrors `cep-idl/v1/manifest.ts`.
    #[serde(default)]
    pub main: Option<String>,

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Engines {
    /// SemVer range string, e.g. `^1.0`.
    pub cronymax: String,
}

/// v1 alpha dropped the OS-level capability gate. The `capabilities` field
/// is still parsed for forward compatibility — older manifests with
/// `fs / network / process / ...` keys still load — but the platform does
/// not enforce any of them.
///
/// The one capability the platform DOES enforce at the RPC layer is
/// `events.subscribe` / `events.emit`: extensions can only subscribe to
/// topics they declared and only emit under their publisher namespace
/// (and only for declared patterns). See [`crate::extensions::events`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Topics this extension may subscribe to and emit on. Both arrays
    /// default to empty; missing the whole `events` key in JSON behaves
    /// the same as `{ "subscribe": [], "emit": [] }`.
    #[serde(default)]
    pub events: EventsCapability,

    /// Anything else under `capabilities.*` is captured here for forward
    /// compatibility but not enforced.
    #[serde(flatten)]
    pub _ignored: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Per-extension event-bus capability whitelist. Mirrors
/// `capabilities.events` in `cep-idl/v1/manifest.ts`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EventsCapability {
    /// Topics this extension may `cronymax.events.on(topic, ...)`. Each
    /// entry is an exact topic string; `cronymax.*` platform topics must
    /// appear here verbatim.
    #[serde(default)]
    pub subscribe: Vec<String>,

    /// Topics this extension may `cronymax.events.emit(topic, ...)`.
    /// Patterns may end with `.*` for prefix matching; `*` alone matches
    /// everything (under the publisher namespace — `cronymax.*` is
    /// always rejected at emit time).
    #[serde(default)]
    pub emit: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandContribution {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigPageContribution {
    pub id: String,
    pub title: String,
    pub entry: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRendererContribution {
    pub id: String,
    #[serde(rename = "mimeTypes")]
    pub mime_types: Vec<String>,
    pub entry: String,
    /// Optional CSP overrides for the renderer iframe. Currently scoped
    /// to `connect-src` host allowlisting. Mirrors `RendererCsp` in
    /// `cep-idl/v1/manifest.ts`.
    ///
    /// The default iframe CSP applied by the `cronymax-webview://` scheme
    /// handler blocks all outbound network (`connect-src 'self'`). To let
    /// the renderer fetch from external hosts, declare them here; the
    /// scheme handler merges them into the response CSP header.
    ///
    /// This is INDEPENDENT of the extension's Node-side
    /// `capabilities.network` — Node fetch and iframe fetch are separate
    /// origins, and each must be authorised in its own dimension.
    #[serde(default)]
    pub csp: Option<RendererCsp>,
}

/// CSP customisations for a content renderer's iframe. Mirrors the IDL
/// `RendererCsp` in `cep-idl/v1/manifest.ts`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererCsp {
    /// Hosts merged into the iframe's `connect-src` CSP directive
    /// (e.g. `["https://api.example.com"]`). Empty / absent = no
    /// outbound network beyond `self`.
    #[serde(default, rename = "connect_src")]
    pub connect_src: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarViewContribution {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub entry: String,
}

impl Manifest {
    /// Parse the on-disk JSON. Pure syntactic deserialization; semantic
    /// checks live in [`Manifest::validate`].
    pub fn from_json(raw: &str) -> ExtensionResult<Self> {
        serde_json::from_str(raw).map_err(|e| ExtensionError::ManifestParse(e.to_string()))
    }

    /// Run schema-level checks. v1 alpha dropped the OS capability gate
    /// (see `permission-removal.md`), so the rules collapse to four:
    ///
    /// 1. Required string fields are non-empty
    /// 2. `id == "<publisher>.<name>"` and `id`'s first segment matches
    ///    `publisher`
    /// 3. The `cronymax` publisher / `cronymax.*` namespace is reserved
    ///    for the platform (the only namespace that's still gated; it
    ///    sits at the RPC routing layer, not OS-level)
    /// 4. Every entry in `activationEvents` parses as a known activation
    ///    event
    pub fn validate(&self, _ext_dir: &std::path::Path) -> ExtensionResult<()> {
        self.validate_required_fields()?;
        self.validate_id_format()?;
        self.validate_namespace_reserved()?;
        self.validate_activation_events()?;
        Ok(())
    }

    fn validate_activation_events(&self) -> ExtensionResult<()> {
        super::activation::parse_all(&self.activation_events).map(|_| ())
    }

    fn validate_required_fields(&self) -> ExtensionResult<()> {
        fn require(field: &str, value: &str) -> ExtensionResult<()> {
            if value.is_empty() {
                Err(ExtensionError::RequiredFieldMissing(field.into()))
            } else {
                Ok(())
            }
        }
        require("id", &self.id)?;
        require("name", &self.name)?;
        require("version", &self.version)?;
        require("publisher", &self.publisher)?;
        // `main` is optional in v1 (declarative-only extensions). When it
        // IS present it must not be an empty string — otherwise the host
        // spawner would try to load `<ext_dir>/`.
        if let Some(main) = self.main.as_deref() {
            require("main", main)?;
        }
        require("engines.cronymax", &self.engines.cronymax)?;
        Ok(())
    }

    fn validate_id_format(&self) -> ExtensionResult<()> {
        let (pub_part, name_part) = self.id.split_once('.').ok_or_else(|| {
            ExtensionError::ManifestInvalid(format!(
                "id `{}` must be `<publisher>.<name>` (got no `.`)",
                self.id
            ))
        })?;
        if pub_part != self.publisher {
            return Err(ExtensionError::PublisherPrefixMismatch {
                id: self.id.clone(),
                publisher: self.publisher.clone(),
            });
        }
        if name_part.is_empty() {
            return Err(ExtensionError::ManifestInvalid(format!(
                "id `{}` is missing the `<name>` segment after `.`",
                self.id
            )));
        }
        Ok(())
    }

    /// Only the publisher reservation survived the v1-alpha capability
    /// drop. `cronymax` is platform-owned; third-party extensions can't
    /// claim that publisher (and therefore can't take `cronymax.*` ids).
    fn validate_namespace_reserved(&self) -> ExtensionResult<()> {
        if self.publisher == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(
                "publisher `cronymax` is reserved".into(),
            ));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_json() -> &'static str {
        r#"{
            "id": "alice.minimal",
            "name": "Minimal",
            "version": "0.0.1",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./main.js",
            "activationEvents": []
        }"#
    }

    #[test]
    fn parses_minimal_manifest() {
        let m = Manifest::from_json(minimal_json()).expect("minimal manifest parses");
        assert_eq!(m.id, "alice.minimal");
        assert_eq!(m.name, "Minimal");
        assert_eq!(m.version, "0.0.1");
        assert_eq!(m.publisher, "alice");
        assert_eq!(m.engines.cronymax, "^1.0");
        assert_eq!(m.main.as_deref(), Some("./main.js"));
        assert!(m.activation_events.is_empty());
        // defaults
        assert!(m.contributes.commands.is_empty());
        assert!(m.extension_dependencies.is_empty());
        // optional metadata
        assert!(m.description.is_none());
        assert!(m.icon.is_none());
        assert!(m.keywords.is_none());
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let err = Manifest::from_json("{ not json").unwrap_err();
        assert!(
            matches!(err, ExtensionError::ManifestParse(_)),
            "expected ManifestParse, got {err:?}",
        );
    }

    #[test]
    fn from_json_uses_manifest_parse_not_json_variant() {
        // Regression check: `From<serde_json::Error>` on `ExtensionError` maps
        // to `Json(...)`, but `from_json` must surface `ManifestParse(...)` so
        // callers can distinguish manifest-shaped failures from other JSON
        // I/O in the platform.
        let err = Manifest::from_json("{").unwrap_err();
        match err {
            ExtensionError::ManifestParse(msg) => assert!(!msg.is_empty()),
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_top_level_fields_are_tolerated() {
        // Forward-compat: a manifest written against a future v1.x patch
        // release MUST still parse on an older runtime. Validation
        // (`P1-T02`) is where strict checks happen.
        let raw = r#"{
            "id": "alice.fwd",
            "name": "Fwd",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": [],
            "someFutureField": { "anything": 42 }
        }"#;
        let m = Manifest::from_json(raw).expect("forward-compat manifest parses");
        assert_eq!(m.id, "alice.fwd");
    }

    #[test]
    fn legacy_capabilities_block_parses_but_is_inert() {
        // v1 alpha dropped the OS capability gate. Old manifests that
        // carry `capabilities.{fs,network,process,secrets,...}` must
        // still parse cleanly so existing extensions don't break; the
        // platform just ignores the content.
        let raw = r#"{
            "id": "alice.legacy",
            "name": "Legacy",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": [],
            "capabilities": {
                "fs":               [{ "path": "{WORKSPACE}", "mode": "rw" }],
                "network":          { "allow": ["api.openai.com"] },
                "process":          true,
                "workers":          false,
                "native_addons":    false,
                "secrets":          { "namespace": "alice.legacy.*" },
                "events.subscribe": ["cronymax.message.assistant.done"],
                "events.emit":      ["alice.legacy.*"],
                "ui-slots":         ["sidebar"],
                "extension-points": ["cronymax.command"],
                "auth.providers":   ["oauth-generic"]
            }
        }"#;
        let m = Manifest::from_json(raw).expect("legacy capabilities still parse");
        m.validate(std::path::Path::new("/tmp/fake"))
            .expect("legacy capabilities are inert; validation must pass");
    }

    #[test]
    fn contributes_dotted_keys_and_camel_case() {
        let raw = r#"{
            "id": "alice.contrib",
            "name": "Contrib",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": [],
            "contributes": {
                "cronymax.command": [
                    { "id": "alice.contrib.hello", "title": "Hello", "category": "Greetings" }
                ],
                "cronymax.config.schema": {
                    "title": "Alice",
                    "properties": { "x": { "type": "string" } }
                },
                "cronymax.config.page": [
                    { "id": "page.main", "title": "Main", "entry": "./settings.html" }
                ],
                "cronymax.agents.provider": [
                    {
                        "id": "alice.contrib.agent",
                        "label": "Alice Agent",
                        "supportsModels": true,
                        "supportsModes": false,
                        "supportsMcp": true
                    }
                ],
                "cronymax.content.renderer": [
                    { "id": "rend.foo", "mimeTypes": ["text/x-foo", "text/x-bar"], "entry": "./r.html" }
                ],
                "cronymax.ui.sidebar.view": [
                    { "id": "view.main", "title": "Main", "entry": "./view.html" }
                ]
            }
        }"#;
        let m = Manifest::from_json(raw).unwrap();
        let k = &m.contributes;

        assert_eq!(k.commands.len(), 1);
        assert_eq!(k.commands[0].id, "alice.contrib.hello");
        assert_eq!(k.commands[0].category.as_deref(), Some("Greetings"));

        let schema = k.config_schema.as_ref().expect("schema is present");
        assert_eq!(schema.get("title").and_then(|v| v.as_str()), Some("Alice"));

        assert_eq!(k.config_pages.len(), 1);
        assert_eq!(k.config_pages[0].entry, "./settings.html");

        assert_eq!(k.agent_providers.len(), 1);
        let ap = &k.agent_providers[0];
        assert_eq!(ap.id, "alice.contrib.agent");
        assert_eq!(ap.supports_models, Some(true));
        assert_eq!(ap.supports_modes, Some(false));
        assert_eq!(ap.supports_mcp, Some(true));

        assert_eq!(k.content_renderers.len(), 1);
        assert_eq!(
            k.content_renderers[0].mime_types,
            vec!["text/x-foo".to_string(), "text/x-bar".to_string()]
        );

        assert_eq!(k.sidebar_views.len(), 1);
        assert_eq!(k.sidebar_views[0].id, "view.main");
    }

    #[test]
    fn extension_dependencies_round_trip() {
        let raw = r#"{
            "id": "alice.dep",
            "name": "Dep",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": ["onStartup"],
            "extensionDependencies": ["bob.lib", "carol.util"]
        }"#;
        let m = Manifest::from_json(raw).unwrap();
        assert_eq!(m.activation_events, vec!["onStartup".to_string()]);
        assert_eq!(
            m.extension_dependencies,
            vec!["bob.lib".to_string(), "carol.util".to_string()]
        );
    }

    // ── P1-T02 validation tests ────────────────────────────────────────────

    fn validate(json: &str) -> ExtensionResult<Manifest> {
        let m = Manifest::from_json(json)?;
        m.validate(std::path::Path::new("/tmp/fake-ext-dir"))?;
        Ok(m)
    }

    #[test]
    fn validate_accepts_minimal_manifest() {
        // Minimal manifest defines no capabilities / contributes — should
        // pass clean.
        validate(minimal_json()).expect("minimal manifest validates");
    }

    #[test]
    fn validate_rejects_empty_id() {
        let raw = r#"{
            "id": "",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js"
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::RequiredFieldMissing(ref f) if f == "id"),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_missing_main() {
        let raw = r#"{
            "id": "alice.x",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": ""
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::RequiredFieldMissing(ref f) if f == "main"),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_missing_engines() {
        let raw = r#"{
            "id": "alice.x",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "" },
            "main": "./m.js"
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::RequiredFieldMissing(ref f) if f == "engines.cronymax"),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_publisher_mismatch() {
        let raw = r#"{
            "id": "bob.bar",
            "name": "Bar",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js"
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::PublisherPrefixMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_id_without_dot() {
        let raw = r#"{
            "id": "nodotted",
            "name": "X",
            "version": "0.1.0",
            "publisher": "nodotted",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js"
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::ManifestInvalid(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_id_with_empty_name_segment() {
        let raw = r#"{
            "id": "alice.",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js"
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::ManifestInvalid(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_cronymax_publisher() {
        let raw = r#"{
            "id": "cronymax.builtin",
            "name": "Builtin",
            "version": "0.1.0",
            "publisher": "cronymax",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js"
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::NamespaceReserved(_)),
            "got {err:?}"
        );
    }

    // Capability-block tests were dropped in the v1-alpha permission-model
    // removal. Manifests with arbitrary `capabilities.{fs,network,...}`
    // shapes parse but the platform no longer interprets them.

    #[test]
    fn validate_rejects_unknown_activation_event() {
        let raw = r#"{
            "id": "alice.x",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": ["onLaunch"]
        }"#;
        let err = validate(raw).unwrap_err();
        assert!(
            matches!(err, ExtensionError::UnknownActivationEvent(ref e) if e == "onLaunch"),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_accepts_known_activation_events() {
        let raw = r#"{
            "id": "alice.x",
            "name": "X",
            "version": "0.1.0",
            "publisher": "alice",
            "engines": { "cronymax": "^1.0" },
            "main": "./m.js",
            "activationEvents": [
                "onStartup",
                "*",
                "onCommand:alice.x.hi",
                "onAgentProvider:alice.x.agent",
                "onView:alice.x.view"
            ]
        }"#;
        validate(raw).expect("all known prefixes validate");
    }
}
