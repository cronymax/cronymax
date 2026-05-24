//! Composition root for the cronymax runtime.
//!
//! [`RuntimeServices`] groups every shared service required by
//! [`super::handler::RuntimeHandler`] and (eventually) [`super::agent_runner::AgentRunner`].
//! It is constructed once per process and shared via `Arc<RuntimeServices>`.
//!
//! Having a single composition root means:
//! * the telescoping `RuntimeHandler::with_*/with_all` constructors can be collapsed
//!   to `RuntimeHandler::new(Arc<RuntimeServices>)`;
//! * integration tests substitute `MockLlmFactory` / `FakeCapabilityFactory` without
//!   touching any real infrastructure.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::capability::factory::{CapabilityFactory, DefaultCapabilityFactory};
use crate::config::RuntimeConfig;
use crate::extensions::host::node::SpawnConfig;
use crate::extensions::{
    default_bundled_bootstrap, default_bundled_node, default_registry_root, ExtensionRegistry,
    ExtensionRuntime,
};
use crate::flow::{FlowRuntimeOnCreate, FlowRuntimeRegistry};
use crate::llm::factory::{DefaultLlmProviderFactory, LlmProviderFactory};
use crate::memory::MemoryManager;
use crate::protocol::events::RuntimeEventPayload;
use crate::runtime::authority::{flow_run_topics, RuntimeAuthority};
use crate::terminal::SharedPtySessionManager;

// ── RuntimeServices ───────────────────────────────────────────────────────────

/// Shared services injected into every runtime component.
///
/// Construct via [`RuntimeServices::new`] for production, or build manually in
/// tests by supplying mock/fake implementations via the public fields.
pub struct RuntimeServices {
    /// Authority: the single source of truth for runs, agents, permissions, etc.
    pub authority: RuntimeAuthority,

    /// Registry of lazily-initialised [`crate::flow::runtime::FlowRuntime`] instances.
    pub flow_registry: Arc<FlowRuntimeRegistry>,

    /// Factory that creates an [`crate::llm::provider::LlmProvider`] for a given
    /// [`crate::llm::config::LlmConfig`].
    pub llm_factory: Arc<dyn LlmProviderFactory>,

    /// Factory that assembles a `DispatcherBuilder` with tier-appropriate shell
    /// and filesystem capabilities.
    pub capability_factory: Arc<dyn CapabilityFactory>,

    /// Shared PTY session managers — keyed by workspace root string so that
    /// sessions created via the browser transport are visible to the renderer
    /// transport (and vice-versa).
    pub terminal_managers: Arc<Mutex<HashMap<String, SharedPtySessionManager>>>,

    /// Optional semantic-memory manager (present when embedding is configured).
    pub memory_manager: Option<Arc<MemoryManager>>,

    /// Extension platform runtime. Consumers use this to discover and drive
    /// activated extension contribution points.
    pub extensions: Option<ExtensionRuntime>,
}

impl RuntimeServices {
    /// Construct production [`RuntimeServices`] from `config`.
    ///
    /// Builds `DefaultLlmProviderFactory` (with a fresh `CopilotTokenCache`) and
    /// `DefaultCapabilityFactory`, plus a new `FlowRuntimeRegistry`.
    ///
    /// Pass the pre-constructed `authority` (from [`RuntimeAuthority::rehydrate`]),
    /// the shared `terminal_managers` map, and an optional `memory_manager`.
    pub fn new(
        _config: &RuntimeConfig,
        authority: RuntimeAuthority,
        terminal_managers: Arc<Mutex<HashMap<String, SharedPtySessionManager>>>,
        memory_manager: Option<Arc<MemoryManager>>,
    ) -> Arc<Self> {
        let llm_factory: Arc<dyn LlmProviderFactory> = Arc::new(DefaultLlmProviderFactory::new());
        let capability_factory: Arc<dyn CapabilityFactory> = Arc::new(DefaultCapabilityFactory);

        // Wire the FlowRuntime event emitter to the authority at composition root.
        // Emits to ["flow:{event}", "flow_run:{flow_run_id}"] and also to
        // "session:{sid}" when the flow run has an originating session.
        let auth_for_registry = authority.clone();
        let on_create: FlowRuntimeOnCreate = Arc::new(move |rt| {
            let auth = auth_for_registry.clone();
            rt.set_event_emitter(Box::new(move |event, json_payload| {
                // Extract flow_run_id from the payload JSON ({"run_id": "run-..."}).
                let flow_run_id: String = serde_json::from_str::<serde_json::Value>(json_payload)
                    .ok()
                    .and_then(|v| v.get("run_id").and_then(|r| r.as_str()).map(str::to_owned))
                    .unwrap_or_default();
                let session_id = if flow_run_id.is_empty() {
                    None
                } else {
                    auth.resolve_session(&flow_run_id)
                };
                let mut topics = flow_run_topics(&flow_run_id, session_id.as_deref());
                // Preserve the legacy flat topic for backward compatibility during migration.
                topics.push(format!("flow:{event}"));
                let data = serde_json::json!({ "event": event, "payload": json_payload });
                auth.emit_many(&topics, RuntimeEventPayload::Raw { data });
            }));
        });
        let flow_registry = Arc::new(FlowRuntimeRegistry::with_on_create(on_create));

        let extensions = default_registry_root().map(|extensions_root| {
            let mut extension_registry = ExtensionRegistry::new(extensions_root);
            if let Err(e) = extension_registry.refresh() {
                tracing::warn!(error = %e, "extension registry refresh failed during runtime startup");
            }
            // Snapshot ids to activate before moving the registry into the
            // runtime. Filtering on `enabled` here avoids spawning hosts for
            // extensions the user explicitly disabled via the CLI.
            let to_activate: Vec<String> = extension_registry
                .iter()
                .filter(|e| e.enabled)
                .map(|e| e.manifest.id.clone())
                .collect();
            let runtime = ExtensionRuntime::new(extension_registry);

            // Bridge webview registry events into the authority's
            // "extensions/webview" topic so the C++ BridgeHandler can
            // subscribe and forward `Message` events to the matching
            // iframe via `kMsgWebviewDeliver`. The PanelCreated /
            // PanelDisposed / VisibilityChanged variants are surfaced
            // here too so the renderer UI can react to platform-side
            // panel lifecycle without polling.
            let auth_for_webview = authority.clone();
            runtime.set_webview_emitter(std::sync::Arc::new(move |event| {
                let payload = match serde_json::to_value(&event) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "failed to serialise WebviewEvent",
                        );
                        return;
                    }
                };
                auth_for_webview.emit(
                    "extensions/webview",
                    RuntimeEventPayload::Raw { data: payload },
                );
            }));

            spawn_startup_activation(&runtime, &to_activate);
            runtime
        });

        Arc::new(Self {
            authority,
            flow_registry,
            llm_factory,
            capability_factory,
            terminal_managers,
            memory_manager,
            extensions,
        })
    }

    /// Minimal constructor for legacy code paths that don't have a `RuntimeConfig`.
    /// Uses default factories (same as `new`) but skips config-dependent wiring.
    pub fn new_minimal(
        authority: RuntimeAuthority,
        terminal_managers: Arc<Mutex<HashMap<String, SharedPtySessionManager>>>,
    ) -> Arc<Self> {
        let llm_factory: Arc<dyn LlmProviderFactory> = Arc::new(DefaultLlmProviderFactory::new());
        let capability_factory: Arc<dyn CapabilityFactory> = Arc::new(DefaultCapabilityFactory);
        let flow_registry = Arc::new(FlowRuntimeRegistry::default());
        Arc::new(Self {
            authority,
            flow_registry,
            llm_factory,
            capability_factory,
            terminal_managers,
            memory_manager: None,
            extensions: None,
        })
    }
}

/// Spawn an async activation task per enabled extension. Failures are
/// logged but never block startup — a single broken extension shouldn't
/// take down the rest of the runtime.
///
/// Requires a tokio runtime in scope; in environments without one
/// (e.g. the synchronous `Runtime::new` unit tests in `lifecycle.rs`)
/// this becomes a no-op so we don't panic at composition time.
fn spawn_startup_activation(runtime: &ExtensionRuntime, ext_ids: &[String]) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        if !ext_ids.is_empty() {
            tracing::debug!(
                count = ext_ids.len(),
                "no tokio runtime in scope; skipping extension startup activation",
            );
        }
        return;
    };

    let bundled_node = default_bundled_node();
    let bootstrap_js = default_bundled_bootstrap();
    let (bundled_node, bootstrap_js) = match (bundled_node, bootstrap_js) {
        (Some(n), Some(b)) if n.is_file() && b.is_file() => (n, b),
        _ => {
            if !ext_ids.is_empty() {
                tracing::warn!(
                    count = ext_ids.len(),
                    "bundled Node 26 or bootstrap.js not found; \
                     skipping extension startup activation. \
                     Set CRONYMAX_BUNDLED_DIR or run scripts/fetch-node26.sh.",
                );
            }
            return;
        }
    };

    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let Some(home) = home else {
        tracing::warn!(
            "neither HOME nor USERPROFILE is set; skipping extension startup activation",
        );
        return;
    };

    for ext_id in ext_ids {
        let runtime = runtime.clone();
        let ext_id = ext_id.clone();
        let node_binary = bundled_node.clone();
        let bootstrap_js = bootstrap_js.clone();
        let storage_dir = home
            .join(".cronymax")
            .join("extensions")
            .join(&ext_id)
            .join("storage");
        let global_storage_dir = home.join(".cronymax").join("global-state").join(&ext_id);

        handle.spawn(async move {
            if let Err(e) = std::fs::create_dir_all(&storage_dir) {
                tracing::warn!(
                    ext_id = %ext_id,
                    path = %storage_dir.display(),
                    error = %e,
                    "failed to create extension storage dir; activation may still succeed",
                );
            }
            if let Err(e) = std::fs::create_dir_all(&global_storage_dir) {
                tracing::warn!(
                    ext_id = %ext_id,
                    path = %global_storage_dir.display(),
                    error = %e,
                    "failed to create extension global-storage dir; activation may still succeed",
                );
            }

            let result = {
                let ext_id_for_cfg = ext_id.clone();
                runtime
                    .activate(&ext_id, move |_manifest, ext_dir| SpawnConfig {
                        ext_id: ext_id_for_cfg,
                        node_binary,
                        node_flags: vec!["--no-warnings".into()],
                        bootstrap_js,
                        ext_dir: ext_dir.clone(),
                        storage_dir,
                        global_storage_dir,
                        workspace_dirs: Vec::new(),
                        manifest_path: ext_dir.join("cronymax-extension.json"),
                        max_restarts: 0,
                        ping_interval: Some(SpawnConfig::ping_interval_default()),
                    })
                    .await
            };

            match result {
                Ok(()) => {
                    tracing::info!(ext_id = %ext_id, "extension activated at startup");
                }
                Err(e) => {
                    tracing::warn!(
                        ext_id = %ext_id,
                        error = %e,
                        "extension activation failed at startup; other extensions unaffected",
                    );
                }
            }
        });
    }
}
