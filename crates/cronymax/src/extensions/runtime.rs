//! Top-level extension orchestrator.
//!
//! `ExtensionRuntime` is the single object the rest of cronymax (chat
//! panel, flow runtime, settings UI) talks to. It owns:
//!
//! * One [`ExtensionRegistry`][crate::extensions::registry::ExtensionRegistry]
//!   describing what's installed and which are enabled
//! * One [`ContributionRegistry`][crate::extensions::contributions::ContributionRegistry]
//!   reflecting every `contributes.*` from every manifest
//! * One [`AgentProviderRegistry`][crate::extensions::api::agents::AgentProviderRegistry],
//!   [`CommandRegistry`][crate::extensions::api::commands::CommandRegistry],
//!   [`ContentRendererRegistry`][crate::extensions::api::renderers::ContentRendererRegistry],
//!   [`SidebarViewRegistry`][crate::extensions::api::sidebar::SidebarViewRegistry],
//!   and [`LifecycleState`][crate::extensions::api::lifecycle::LifecycleState]
//!   shared across all activated extensions
//! * A map of `ext_id → ExtensionHandle` — the live [`NodeHost`] plus
//!   the [`Arc<Connection>`] used to send RPC to that extension
//!
//! ### `state.handles` is the single source of truth for RPC channels
//!
//! Every L2 EP registry entry is **pure metadata** — `ProviderEntry`,
//! `RendererEntry`, `SidebarViewEntry` do not carry an `Arc<Connection>`.
//! Callers (chat panel, flow runtime) look up the entry, then call
//! [`Self::send_to_extension`] / [`Self::notify_extension`] passing the
//! `owning_ext` id; the runtime joins to the live conn via `state.handles`.
//!
//! This is the keystone fix for the chicken-and-egg that earlier shipped
//! as a `LateConn` slot: handlers used to need to capture a connection
//! that didn't exist yet, so we threaded it in after spawn. By keeping
//! the conn off the entry, handler closures only need shared state and
//! metadata — no late binding required.
//!
//! ### `activate()` flow
//!
//! 1. Look up the manifest from the registry (errors: NotInstalled,
//!    NotEnabled, AlreadyActivated).
//! 2. Ingest its `contributes.*` into the contribution registry so the
//!    settings UI can see the EP even if activate() throws later.
//! 3. Build an [`RpcServer`] with per-extension notify handlers — each
//!    closure captures `ext_id`, the manifest, and a clone of
//!    `state` so it can read the live conn (when needed for error
//!    feedback) and write to the matching shared registry.
//! 4. Spawn a [`NodeHost`] with that RPC table.
//! 5. Snapshot the connection and insert the `ExtensionHandle` into
//!    `state.handles` **before** driving `extension/activate`. This
//!    makes the conn visible to any register notify the extension fires
//!    from inside its activate() callback.
//! 6. Send `extension/activate` request and await the response. On
//!    failure, roll back: remove the handle, drop contributions, kill
//!    the host.
//! 7. Mark the [`LifecycleState`] so duplicate activate is rejected.
//!
//! [`Self::deactivate`] runs the reverse:
//!
//! 1. Send `extension/deactivate` request (best-effort; failure tolerated)
//! 2. Drop the handle (severs the conn reference)
//! 3. Clear every shared registry of this extension's contributions
//! 4. Mark lifecycle deactivated
//! 5. Shut down the host
//!
//! ### `extension/registerError` reverse notify
//!
//! When a register-notify handler fails (e.g. extension tries to
//! register a provider id not in its manifest), the platform sends an
//! `extension/registerError` notify back to the extension carrying
//! `{ ep, id, reason }`. The bootstrap.js SDK shim turns this into a
//! `console.error` so the developer sees it; the alternative — silent
//! drop — left mistakes invisible until users complained.
//!
//! ### Test note
//!
//! Unit tests in this module exercise the wiring using duplex
//! `Connection` pairs (no Node binary). End-to-end activation against
//! a real Node 26 process is covered in
//! `tests/p4_extension_runtime_e2e.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rmpv::Value;

use crate::extensions::api::agents::{
    AgentProviderRegistry, AgentSessionEvent, AgentSessionRouter, ProviderEntry,
};
use crate::extensions::api::commands::CommandRegistry;
use crate::extensions::api::lifecycle::LifecycleState;
use crate::extensions::api::renderers::{
    ContentRendererRegistry, RendererEvent, RendererEventEmitter,
};
use crate::extensions::api::sidebar::{SidebarViewEntry, SidebarViewRegistry};
use crate::extensions::api::webview::{
    CreatePanelArgs, PanelSlot, WebviewEventEmitter, WebviewRegistry,
};
use crate::extensions::contributions::ContributionRegistry;
use crate::extensions::error::{ExtensionError, ExtensionResult};
#[cfg(test)]
use crate::extensions::events::PlatformTopic;
use crate::extensions::events::{EventBus, EventPayload, SubscriptionGuard};
use crate::extensions::host::node::{NodeHost, SpawnConfig};
use crate::extensions::manifest::{AgentProviderContribution, Manifest, SidebarViewContribution};
use crate::extensions::registry::ExtensionRegistry;
use crate::extensions::rpc::codec::{
    agents_method, method, sidebar_method, webview_method, webview_view_method,
};
use crate::extensions::rpc::{Connection, RpcServer};

/// Live state for one activated extension. Held inside `ExtensionRuntime`
/// while the host is up; dropped on deactivate. The ext_id is implicit
/// in the handle's key in `RuntimeState::handles`.
#[derive(Debug)]
struct ExtensionHandle {
    host: NodeHost,
    /// RPC channel to this extension's host. Cheap to clone (`Arc`).
    /// Looked up by [`ExtensionRuntime::send_to_extension`] and
    /// [`ExtensionRuntime::notify_extension`].
    conn: Arc<Connection>,
    /// Per-extension event-bus subscription guards. Each guard
    /// corresponds to one `cronymax.events.on(topic, …)` call from the
    /// extension; dropping the vector on deactivate severs every
    /// forwarding listener in one shot. Keyed by topic so duplicate
    /// `on(topic, …)` calls compose instead of replace.
    event_subscriptions: HashMap<String, Vec<SubscriptionGuard>>,
}

/// Top-level orchestrator. Cheap to clone (`Arc` internals).
#[derive(Clone, Debug)]
pub struct ExtensionRuntime {
    state: Arc<RuntimeState>,
}

/// Composition-root callback fired when the contribution registry changes
/// (extension activate / deactivate). A bare signal — the UI refetches the
/// full snapshot. No payload keeps it cheap and avoids leaking descriptor
/// shape into the emit path.
pub type ContributionsChangedEmitter = Arc<dyn Fn() + Send + Sync>;

struct RuntimeState {
    registry: Mutex<ExtensionRegistry>,
    contributions: Mutex<ContributionRegistry>,
    lifecycle: Mutex<LifecycleState>,
    commands: Mutex<CommandRegistry>,
    providers: AgentProviderRegistry,
    renderers: ContentRendererRegistry,
    sidebars: SidebarViewRegistry,
    /// Live webview panel index. Created via `webview/createPanel` RPC,
    /// torn down via `webview/disposePanel` or on extension deactivate.
    /// The renderer subscribes to its event stream via the emitter wired
    /// at composition root.
    webviews: WebviewRegistry,
    /// Per-extension live host. Insert immediately after spawn (so
    /// register notifies fired during activate() can find the conn);
    /// remove on deactivate or activate-failure rollback.
    handles: Mutex<HashMap<String, ExtensionHandle>>,
    /// Routes inbound `agents/event` and `agents/turn.done` notifies
    /// from any extension host to the in-flight chat / flow dispatcher
    /// that holds the corresponding `session_id`. Populated by the
    /// dispatcher right before `agents/session.prompt`, drained by the
    /// dispatcher on run completion. Shared across extensions because
    /// the wire format routes by sessionId, not by owning_ext.
    session_router: AgentSessionRouter,
    /// L1.5 platform-event bus. Emit-from-platform sites in the chat
    /// dispatcher / tool runtime fan out `cronymax.*` topics here;
    /// extensions subscribe to topics they declared in
    /// `capabilities.events.subscribe` via `events/subscribe` RPC.
    events: EventBus,
    /// Composition-root-installed callback that pipes
    /// [`RendererEvent`]s into the [`RuntimeAuthority`] topic
    /// `extensions/renderer`. Defaults to a no-op so unit tests can
    /// drive the runtime without a renderer attached.
    renderer_event_emitter: RwLock<RendererEventEmitter>,
    /// Composition-root callback fired on contribution-registry changes;
    /// pipes into the `extensions/contributions` authority topic so the
    /// activity-bar rail refetches. No-op default for tests.
    contributions_changed_emitter: RwLock<ContributionsChangedEmitter>,
}

impl std::fmt::Debug for RuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-written because `renderer_event_emitter` holds a dyn Fn
        // closure that doesn't implement Debug. Every other field
        // forwards normally.
        f.debug_struct("RuntimeState")
            .field("registry", &self.registry)
            .field("contributions", &self.contributions)
            .field("lifecycle", &self.lifecycle)
            .field("commands", &self.commands)
            .field("providers", &self.providers)
            .field("renderers", &self.renderers)
            .field("sidebars", &self.sidebars)
            .field("webviews", &self.webviews)
            .field("handles", &self.handles)
            .field("session_router", &self.session_router)
            .field("events", &self.events)
            .field("renderer_event_emitter", &"<fn>")
            .field("contributions_changed_emitter", &"<fn>")
            .finish()
    }
}

impl ExtensionRuntime {
    pub fn new(registry: ExtensionRegistry) -> Self {
        Self {
            state: Arc::new(RuntimeState {
                registry: Mutex::new(registry),
                contributions: Mutex::new(ContributionRegistry::new()),
                lifecycle: Mutex::new(LifecycleState::new()),
                commands: Mutex::new(CommandRegistry::new()),
                providers: AgentProviderRegistry::new(),
                renderers: ContentRendererRegistry::new(),
                sidebars: SidebarViewRegistry::new(),
                webviews: WebviewRegistry::new(),
                handles: Mutex::new(HashMap::new()),
                session_router: AgentSessionRouter::new(),
                events: EventBus::new(),
                renderer_event_emitter: RwLock::new(Arc::new(|_| {})),
                contributions_changed_emitter: RwLock::new(Arc::new(|| {})),
            }),
        }
    }

    // ── accessors used by the rest of cronymax (chat panel / flow / UI) ──

    pub fn providers(&self) -> &AgentProviderRegistry {
        &self.state.providers
    }

    pub fn renderers(&self) -> &ContentRendererRegistry {
        &self.state.renderers
    }

    pub fn sidebars(&self) -> &SidebarViewRegistry {
        &self.state.sidebars
    }

    /// Live webview panel registry. The chat / settings / sidebar shells
    /// subscribe to its event stream via the emitter wired at
    /// composition root (see [`Self::set_webview_emitter`]).
    pub fn webviews(&self) -> &WebviewRegistry {
        &self.state.webviews
    }

    /// Replace the renderer-facing webview event emitter. Called once
    /// at composition root so events flow into the [`RuntimeAuthority`]
    /// topic the panel UI shells subscribe to.
    pub fn set_webview_emitter(&self, emitter: WebviewEventEmitter) {
        self.state.webviews.set_emitter(emitter);
    }

    /// Replace the content-renderer event emitter. Called once at
    /// composition root so [`RendererEvent`]s flow into the
    /// [`RuntimeAuthority`] topic `extensions/renderer` the chat surface
    /// subscribes to. Test runtimes without a renderer keep the
    /// no-op default.
    pub fn set_renderer_emitter(&self, emitter: RendererEventEmitter) {
        *self.state.renderer_event_emitter.write() = emitter;
    }

    fn emit_renderer_event(&self, event: RendererEvent) {
        // Clone the emitter under read lock then drop the lock before
        // firing — mirrors WebviewRegistry::emit so emitters that
        // re-enter the runtime don't deadlock.
        let emitter = self.state.renderer_event_emitter.read().clone();
        emitter(event);
    }

    /// Replace the contribution-changed emitter. Called once at composition
    /// root so registry changes flow into the `extensions/contributions`
    /// authority topic the activity-bar rail subscribes to.
    pub fn set_contributions_emitter(&self, emitter: ContributionsChangedEmitter) {
        *self.state.contributions_changed_emitter.write() = emitter;
    }

    fn emit_contributions_changed(&self) {
        let emitter = self.state.contributions_changed_emitter.read().clone();
        emitter();
    }

    /// L1.5 platform-event bus. Chat / tool dispatch sites call
    /// [`EventBus::emit_from_platform`] on this to fan out
    /// `cronymax.*` topics to subscribed extensions.
    pub fn events(&self) -> &EventBus {
        &self.state.events
    }

    /// Streaming session event router. Chat / flow dispatchers call
    /// [`AgentSessionRouter::register`] before sending
    /// `agents/session.prompt`, then iterate the returned receiver until a
    /// `TurnDone` / `Done` arrives. Inbound `agents/event` and
    /// `agents/turn.done` notify handlers (registered per-extension by
    /// [`Self::build_rpc_server`]) push into the same router.
    pub fn session_router(&self) -> &AgentSessionRouter {
        &self.state.session_router
    }

    /// Snapshot of every contribution currently ingested, scoped to one
    /// kind. Used by the integration tests and the settings UI to list
    /// e.g. all `cronymax.command` entries. Each element is the
    /// descriptor's `metadata` JSON payload — the per-kind shape (e.g.
    /// `AgentProviderContribution` for `cronymax.agents.provider`).
    pub fn contributions_for_ep(&self, kind: &str) -> Vec<serde_json::Value> {
        self.state
            .contributions
            .lock()
            .for_kind(kind)
            .map(|d| d.metadata.clone())
            .collect()
    }

    /// Full snapshot of every descriptor across every kind. Used by the
    /// `contribution/list` IPC variant.
    pub fn contributions_snapshot(
        &self,
    ) -> Vec<crate::extensions::contributions::ContributionDescriptor> {
        self.state.contributions.lock().snapshot()
    }

    /// Programmatically add one platform / workspace descriptor. Returns
    /// any previous descriptor at the same `(kind, id)`.
    pub fn add_contribution(
        &self,
        descriptor: crate::extensions::contributions::ContributionDescriptor,
    ) -> Option<crate::extensions::contributions::ContributionDescriptor> {
        self.state.contributions.lock().add(descriptor)
    }

    /// Programmatically remove one descriptor by `(kind, id)`.
    pub fn remove_contribution(
        &self,
        kind: &str,
        id: &str,
    ) -> Option<crate::extensions::contributions::ContributionDescriptor> {
        self.state.contributions.lock().remove(kind, id)
    }

    /// Look up one descriptor by `(kind, id)` and return an owned clone.
    pub fn contribution_get(
        &self,
        kind: &str,
        id: &str,
    ) -> Option<crate::extensions::contributions::ContributionDescriptor> {
        self.state.contributions.lock().get(kind, id).cloned()
    }

    /// Find the first descriptor with matching `id` across all kinds.
    pub fn contribution_find_by_id(
        &self,
        id: &str,
    ) -> Option<crate::extensions::contributions::ContributionDescriptor> {
        self.state.contributions.lock().find_by_id(id).cloned()
    }

    pub fn is_activated(&self, ext_id: &str) -> bool {
        self.state.lifecycle.lock().is_activated(ext_id)
    }

    /// Test-only: synthesise an activation record without spinning up a Node
    /// host. Used by `RuntimeHandler` tests that need an "active extension"
    /// shape without exercising the full `activate()` pipeline.
    #[cfg(test)]
    pub(crate) fn test_mark_activated(&self, ext_id: &str) {
        let _ = self.state.lifecycle.lock().mark_activated(ext_id);
    }

    pub fn activated_ids(&self) -> Vec<String> {
        self.state
            .lifecycle
            .lock()
            .activated_ids()
            .into_iter()
            .map(String::from)
            .collect()
    }

    // ── extension communication entry points ─────────────────────────

    /// Send an RPC request to the named extension's host and await the
    /// response. The extension must currently have a live host (i.e.
    /// `activate()` succeeded and `deactivate()` hasn't fired); otherwise
    /// `NotActivated` is returned without contacting any RPC machinery.
    ///
    /// This is the canonical entry point from chat panel / flow runtime /
    /// UI code that needs to drive an L2 EP — `agents/session.create`,
    /// `agents/session.prompt`, `renderers/render`, etc.
    pub async fn send_to_extension(
        &self,
        ext_id: &str,
        method: &str,
        params: Value,
    ) -> ExtensionResult<Value> {
        let conn = self.conn_for(ext_id)?;
        conn.request(method, params).await
    }

    /// Send an RPC notify to the named extension's host. Fire-and-forget
    /// — no response is awaited. Same activation precondition as
    /// [`Self::send_to_extension`].
    pub async fn notify_extension(
        &self,
        ext_id: &str,
        method: &str,
        params: Value,
    ) -> ExtensionResult<()> {
        let conn = self.conn_for(ext_id)?;
        conn.notify(method, params).await
    }

    fn conn_for(&self, ext_id: &str) -> ExtensionResult<Arc<Connection>> {
        self.state
            .handles
            .lock()
            .get(ext_id)
            .map(|h| h.conn.clone())
            .ok_or_else(|| ExtensionError::NotActivated(ext_id.to_string()))
    }

    /// Test-only: build the per-extension RPC server without going
    /// through spawn. Used by the unit tests that drive register flows
    /// over duplex Connection pairs.
    #[cfg(test)]
    pub(crate) fn build_per_extension_handlers(
        &self,
        ext_id: &str,
        manifest: &Manifest,
    ) -> RpcServer {
        self.build_rpc_server(ext_id.to_string(), manifest.clone())
    }

    /// Test-only: install a `NodeHost`-less handle so cross-module tests
    /// (e.g. `runtime::ext_dispatch`) can exercise the
    /// `send_to_extension` → duplex-peer path without spawning a real
    /// Node subprocess. Mirrors what `activate()` does after spawn, minus
    /// the lifecycle bookkeeping.
    #[cfg(test)]
    pub(crate) fn install_test_handle(&self, ext_id: &str, conn: Arc<Connection>) {
        self.state.handles.lock().insert(
            ext_id.to_string(),
            ExtensionHandle {
                host: NodeHost::dummy_for_test(ext_id),
                conn,
                event_subscriptions: HashMap::new(),
            },
        );
    }

    // ── activate / deactivate ──────────────────────────────────────────

    /// Activate the installed extension `ext_id`. The Node host is
    /// spawned using a [`SpawnConfig`] derived from the registry; callers
    /// fill in the binary / bootstrap paths via the `cfg_builder` callback.
    ///
    /// Errors:
    /// * `NotInstalled` — `ext_id` not in the registry
    /// * `NotEnabled` — registry entry has `enabled == false`
    /// * `AlreadyActivated` — extension is already in `LifecycleState`
    /// * `HostSpawn` — `NodeHost::spawn` failed
    /// * any error from the extension's `activate()` callback (surfaced
    ///   as an `Rpc` error)
    pub async fn activate<F>(&self, ext_id: &str, cfg_builder: F) -> ExtensionResult<()>
    where
        F: FnOnce(&Manifest, PathBuf) -> SpawnConfig,
    {
        let (manifest, ext_dir) = {
            let reg = self.state.registry.lock();
            let entry = reg
                .get(ext_id)
                .ok_or_else(|| ExtensionError::NotInstalled(ext_id.to_string()))?;
            if !entry.enabled {
                return Err(ExtensionError::NotEnabled(ext_id.to_string()));
            }
            (entry.manifest.clone(), entry.ext_dir.clone())
        };

        if self.state.lifecycle.lock().is_activated(ext_id) {
            return Err(ExtensionError::AlreadyActivated(ext_id.to_string()));
        }

        // 1. Ingest contributions immediately — they survive even if
        //    activate() throws, so the settings UI can still surface them.
        self.state.contributions.lock().ingest(&manifest);
        self.emit_contributions_changed();

        // 1.b. Populate the manifest-driven typed registries. Currently this
        //      is just content renderers; agent providers and sidebar views
        //      go through Node-side notify register/unregister because they
        //      need a live RPC handler attached (`session.create`,
        //      `sidebar/view.message`). Renderers don't — they're iframe-
        //      hosted (`acquireCronymaxRendererApi()` in the iframe), so
        //      manifest declaration is the only thing the platform needs.
        if let Err(e) = self.state.renderers.ingest_manifest(&manifest, ext_id) {
            tracing::warn!(
                ext_id = %ext_id,
                error = %e,
                "ingest_manifest for content renderers failed; declared renderers won't dispatch",
            );
        }

        // 1a. Register event-bus capability whitelist so any `events/subscribe`
        //     notify the extension fires from activate() can pass capability
        //     gating. Mirrors contributions: keep the cap registered even if
        //     activate() throws — `deactivate()` is what tears it down. The
        //     ingestion is cheap (a HashSet insert), so the cost is fine.
        if let Err(e) = self.state.events.register_extension(
            ext_id,
            &manifest.capabilities.events.subscribe,
            &manifest.capabilities.events.emit,
        ) {
            tracing::warn!(
                ext_id = %ext_id,
                error = %e,
                "events bus register_extension failed; events.on/.emit will be denied",
            );
        }

        // 2-5. Spawn Node host only when the manifest declares a `main`
        //      entry point. Declarative-only extensions (e.g. a content
        //      renderer with no Node-side coordinator — see P6.5 IDL D7)
        //      skip the entire host pipeline; contributions already
        //      ingested at step 1 are all the platform needs.
        if manifest.main.is_some() {
            // 2. Build the per-extension RPC handler table.
            let rpc = self.build_rpc_server(ext_id.to_string(), manifest.clone());

            // 3. Spawn the host.
            let cfg = cfg_builder(&manifest, ext_dir);
            let host = match NodeHost::spawn(cfg, rpc).await {
                Ok(h) => h,
                Err(e) => {
                    self.state.contributions.lock().remove_extension(ext_id);
                    self.state.renderers.unregister_all_for(ext_id);
                    let _ = self.state.events.unregister_extension(ext_id);
                    self.emit_contributions_changed();
                    return Err(e);
                }
            };

            // 4. Snapshot the conn. Must happen before driving
            //    extension/activate so that any register-notify the extension
            //    fires from inside its activate() callback can find the conn
            //    in state.handles.
            let conn = match host.connection().await {
                Some(c) => c,
                None => {
                    self.state.contributions.lock().remove_extension(ext_id);
                    self.state.renderers.unregister_all_for(ext_id);
                    let _ = self.state.events.unregister_extension(ext_id);
                    self.emit_contributions_changed();
                    let _ = host.shutdown().await;
                    return Err(ExtensionError::HostSpawn(
                        "host spawned but connection unavailable".into(),
                    ));
                }
            };
            self.state.handles.lock().insert(
                ext_id.to_string(),
                ExtensionHandle {
                    host,
                    conn: conn.clone(),
                    event_subscriptions: HashMap::new(),
                },
            );

            // 5. Drive extension/activate. Failure is fatal — rollback all
            //    state and tear down the host.
            if let Err(e) = conn.request(method::EXTENSION_ACTIVATE, Value::Nil).await {
                self.rollback_failed_activate(ext_id).await;
                return Err(e);
            }
        } else {
            // Declarative-only extension: no host, no conn, no
            // `extension/activate` RPC. Contributions are already in the
            // registries; mark lifecycle and we're done. Note that
            // `state.handles` deliberately stays without an entry so
            // downstream code (deactivate, `send_to_extension`, etc.) can
            // tell host-backed and declarative-only apart with a simple
            // map lookup.
            tracing::debug!(
                ext_id = %ext_id,
                "activating declarative-only extension (no `manifest.main`); skipping Node host spawn",
            );
        }

        // 6. Mark lifecycle. This is the user-visible "activated" bit;
        //    state.handles being populated only means "host is alive".
        self.state
            .lifecycle
            .lock()
            .mark_activated(ext_id)
            .expect("checked is_activated above");
        Ok(())
    }

    /// Deactivate `ext_id`. Errors with `NotActivated` if the lifecycle
    /// bit is not set. For host-backed extensions this also drives the
    /// `extension/deactivate` RPC and tears down the Node host; for
    /// declarative-only extensions (P6.5 — no `manifest.main`) the
    /// `state.handles` map has no entry, so we skip the host shutdown
    /// and only clean the registries.
    pub async fn deactivate(&self, ext_id: &str) -> ExtensionResult<()> {
        // Source of truth for "is this extension activated" is the
        // lifecycle table, NOT `state.handles` (which is empty for
        // declarative-only extensions).
        if !self.state.lifecycle.lock().is_activated(ext_id) {
            return Err(ExtensionError::NotActivated(ext_id.to_string()));
        }

        // Pop the host-backed handle if present; missing means
        // declarative-only.
        let handle = self.state.handles.lock().remove(ext_id);

        // Best-effort RPC notice on host-backed extensions; failure means
        // the host is already gone, which is fine.
        if let Some(handle) = handle.as_ref() {
            let _ = handle
                .conn
                .request(method::EXTENSION_DEACTIVATE, Value::Nil)
                .await;
        }

        self.state.contributions.lock().remove_extension(ext_id);
        self.state.providers.unregister_all_for(ext_id);
        self.state.commands.lock().unregister_all_for(ext_id);
        self.state.renderers.unregister_all_for(ext_id);
        self.state.sidebars.unregister_all_for(ext_id);
        let _ = self.state.webviews.dispose_all_for(ext_id);
        let _ = self.state.events.unregister_extension(ext_id);
        let _ = self.state.lifecycle.lock().mark_deactivated(ext_id);
        self.emit_contributions_changed();

        if let Some(handle) = handle {
            // Destructure the handle so `event_subscriptions` drops here
            // (severing every forwarding listener) while `host` survives
            // long enough for the explicit shutdown await below.
            let ExtensionHandle {
                host,
                conn: _,
                event_subscriptions,
            } = handle;
            drop(event_subscriptions);
            let _ = host.shutdown().await;
        }
        Ok(())
    }

    async fn rollback_failed_activate(&self, ext_id: &str) {
        let handle = self.state.handles.lock().remove(ext_id);
        self.state.contributions.lock().remove_extension(ext_id);
        self.state.providers.unregister_all_for(ext_id);
        self.state.commands.lock().unregister_all_for(ext_id);
        self.state.renderers.unregister_all_for(ext_id);
        self.state.sidebars.unregister_all_for(ext_id);
        let _ = self.state.webviews.dispose_all_for(ext_id);
        let _ = self.state.events.unregister_extension(ext_id);
        self.emit_contributions_changed();
        if let Some(h) = handle {
            let _ = h.host.shutdown().await;
        }
    }

    // ── plumbing: build the per-extension RPC server ───────────────────

    fn build_rpc_server(&self, ext_id: String, manifest: Manifest) -> RpcServer {
        let providers = self.state.providers.clone();
        let sidebars = self.state.sidebars.clone();
        let state = self.state.clone();

        let mut builder = RpcServer::builder();

        // ── agents/registerProvider ────────────────────────────────────
        {
            let providers = providers.clone();
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            let manifest_c = manifest.clone();
            builder = builder.on_notify(agents_method::REGISTER_PROVIDER, move |params| {
                let providers = providers.clone();
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                let manifest = manifest_c.clone();
                async move {
                    let result = register_provider_handler(&providers, &manifest, &ext_id, &params);
                    report_register_outcome(
                        &state,
                        &ext_id,
                        "cronymax.agents.provider",
                        &params,
                        "providerId",
                        result,
                    )
                    .await
                }
            });
        }

        // ── agents/unregisterProvider ──────────────────────────────────
        {
            let providers = providers.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(agents_method::UNREGISTER_PROVIDER, move |params| {
                let providers = providers.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let provider_id = extract_str_field(&params, "providerId")?;
                    let _ = providers.unregister(&ext_id, &provider_id)?;
                    Ok(())
                }
            });
        }

        // ── agents/event ───────────────────────────────────────────────
        //
        // Streamed AgentEvent payload from the extension's session.prompt
        // iterator. Notify shape: `{ sessionId, event: { kind, ... } }`.
        // We deserialize the inner event into AgentSessionEvent (typed)
        // and forward to the dispatcher via the runtime's session router.
        // Missing sinks are not errors: they happen routinely when a
        // dispatcher cancels mid-turn or when bookkeeping races a final
        // notify — the inbound notify is logged & dropped.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(agents_method::EVENT, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let json = rmpv_to_json(&params);
                    let obj = json.as_object().ok_or_else(|| {
                        ExtensionError::Rpc(format!(
                            "agents/event from `{ext_id}` is not an object: {json}",
                        ))
                    })?;
                    let session_id = obj
                        .get("sessionId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ExtensionError::Rpc(format!(
                                "agents/event from `{ext_id}` missing string sessionId",
                            ))
                        })?
                        .to_string();
                    let event_val = obj.get("event").cloned().ok_or_else(|| {
                        ExtensionError::Rpc(format!(
                            "agents/event from `{ext_id}` missing `event` field",
                        ))
                    })?;
                    let event: AgentSessionEvent =
                        serde_json::from_value(event_val).map_err(|e| {
                            ExtensionError::Rpc(format!(
                                "agents/event from `{ext_id}` failed to decode AgentEvent: {e}",
                            ))
                        })?;
                    match state.session_router.route_event(&session_id, event).await {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::debug!(
                                target = "cronymax::extensions",
                                ext_id = %ext_id,
                                session_id = %session_id,
                                "dropped agents/event: no dispatcher sink registered",
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                target = "cronymax::extensions",
                                ext_id = %ext_id,
                                session_id = %session_id,
                                error = %e,
                                "failed to route agents/event to dispatcher",
                            );
                        }
                    }
                    Ok(())
                }
            });
        }

        // ── agents/turn.done ───────────────────────────────────────────
        //
        // Explicit turn-boundary signal from bootstrap.js after the
        // session.prompt iterator returns. Even when the extension itself
        // emits a `{kind:"done"}` event first, the dispatcher gets a
        // separate `TurnDone` marker so cleanup logic can run unconditionally.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(agents_method::TURN_DONE, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let session_id = extract_str_field(&params, "sessionId")?;
                    match state.session_router.route_turn_done(&session_id).await {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::debug!(
                                target = "cronymax::extensions",
                                ext_id = %ext_id,
                                session_id = %session_id,
                                "dropped agents/turn.done: no dispatcher sink registered",
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                target = "cronymax::extensions",
                                ext_id = %ext_id,
                                session_id = %session_id,
                                error = %e,
                                "failed to route agents/turn.done to dispatcher",
                            );
                        }
                    }
                    Ok(())
                }
            });
        }

        // ── commands/register ──────────────────────────────────────────
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(method::COMMANDS_REGISTER, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let result = (|| {
                        let command_id = extract_str_field(&params, "commandId")?;
                        state.commands.lock().register(&ext_id, &command_id)?;
                        Ok::<(), ExtensionError>(())
                    })();
                    report_register_outcome(
                        &state,
                        &ext_id,
                        "cronymax.command",
                        &params,
                        "commandId",
                        result,
                    )
                    .await
                }
            });
        }

        // ── commands/unregister ────────────────────────────────────────
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(method::COMMANDS_UNREGISTER, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let command_id = extract_str_field(&params, "commandId")?;
                    state.commands.lock().unregister(&ext_id, &command_id)?;
                    Ok(())
                }
            });
        }

        // NB: there is no `renderers/register` / `renderers/unregister`
        // RPC in v1 — content renderers are iframe-hosted and the registry
        // is manifest-driven (see `ContentRendererRegistry::ingest_manifest`
        // called from `activate`). Earlier alpha drafts routed Node-side
        // `cronymax.renderers.registerRenderer(...)` notifies here; that
        // path was removed in P6.5 (IDL decisions D1+D2).

        // ── sidebar/register ───────────────────────────────────────────
        {
            let sidebars = sidebars.clone();
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            let manifest_c = manifest.clone();
            builder = builder.on_notify(sidebar_method::REGISTER, move |params| {
                let sidebars = sidebars.clone();
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                let manifest = manifest_c.clone();
                async move {
                    let result = register_sidebar_handler(&sidebars, &manifest, &ext_id, &params);
                    report_register_outcome(
                        &state,
                        &ext_id,
                        "cronymax.ui.sidebar.view",
                        &params,
                        "viewId",
                        result,
                    )
                    .await
                }
            });
        }

        // ── sidebar/unregister ─────────────────────────────────────────
        {
            let sidebars = sidebars.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(sidebar_method::UNREGISTER, move |params| {
                let sidebars = sidebars.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let view_id = extract_str_field(&params, "viewId")?;
                    let _ = sidebars.unregister(&ext_id, &view_id)?;
                    Ok(())
                }
            });
        }

        // ── events/subscribe ──────────────────────────────────────────
        //
        // The extension's `cronymax.events.on(topic, handler)` SDK call
        // turns into this notify. We install a forwarding listener on the
        // shared `EventBus` that, whenever the topic fires, sends a
        // `events/publish` notify back over this extension's conn so the
        // bootstrap.js shim can dispatch to the user handler.
        //
        // The `SubscriptionGuard` is parked inside the handle's
        // `event_subscriptions` map; on deactivate the whole map drops at
        // once, severing every listener. Duplicate `on(topic, …)` calls
        // from the same extension stack: each yields its own guard so
        // disposing one doesn't take the others down.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(method::EVENTS_SUBSCRIBE, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let topic = extract_str_field(&params, "topic")?;
                    let conn_for_listener = {
                        let handles = state.handles.lock();
                        handles.get(&ext_id).map(|h| h.conn.clone())
                    };
                    let Some(conn) = conn_for_listener else {
                        return Err(ExtensionError::NotActivated(ext_id.clone()));
                    };
                    let topic_for_listener = topic.clone();
                    let guard = state.events.subscribe(&ext_id, &topic, move |payload| {
                        let conn = conn.clone();
                        let frame = build_publish_frame(&topic_for_listener, payload);
                        // notify() is async; spawn so the bus emitter
                        // stays non-blocking on its synchronous fanout.
                        tokio::spawn(async move {
                            if let Err(e) = conn.notify(method::EVENTS_PUBLISH, frame).await {
                                tracing::warn!(
                                    error = %e,
                                    "failed to forward events/publish to extension",
                                );
                            }
                        });
                    })?;
                    state
                        .handles
                        .lock()
                        .get_mut(&ext_id)
                        .ok_or_else(|| ExtensionError::NotActivated(ext_id.clone()))?
                        .event_subscriptions
                        .entry(topic)
                        .or_default()
                        .push(guard);
                    Ok(())
                }
            });
        }

        // ── events/unsubscribe ────────────────────────────────────────
        //
        // Dropping the topic's guard vector severs every listener the
        // extension installed for that topic. Bootstrap.js calls this on
        // `Disposable.dispose()`; deactivate() also wipes everything
        // implicitly via handle drop.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(method::EVENTS_UNSUBSCRIBE, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let topic = extract_str_field(&params, "topic")?;
                    if let Some(handle) = state.handles.lock().get_mut(&ext_id) {
                        handle.event_subscriptions.remove(&topic);
                    }
                    Ok(())
                }
            });
        }

        // ── events/emit (request) ─────────────────────────────────────
        //
        // Extension-side `cronymax.events.emit(topic, data): Promise<void>`.
        // The bus applies the manifest's `events.emit` whitelist and refuses
        // `cronymax.*` topics. Modeled as a request (not notify) so the
        // returned Promise rejects with the capability error — silent
        // drops on emit would hide capability misconfiguration.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.handle(method::EVENTS_EMIT, move |params, _ctx| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let topic = extract_str_field(&params, "topic")?;
                    let data_rmpv = lookup_field(&params, "data").unwrap_or(Value::Nil);
                    let data_json = rmpv_to_json(&data_rmpv);
                    state
                        .events
                        .emit_from_extension(&ext_id, &topic, data_json)?;
                    Ok(Value::Nil)
                }
            });
        }

        // ── webview/createPanel (request) ─────────────────────────────
        //
        // Extension-side `cronymax.window.createWebviewPanel(opts): Promise<WebviewPanel>`.
        // Request, not notify, so the SDK's awaited promise can carry
        // the resolved URL back to the JS side — the panel JS needs
        // the URL it'll be loaded from for any debug logging /
        // self-introspection, and an `id` collision must surface as a
        // rejection.
        //
        // Response shape: `{ panelId, url, slot }` — the URL is the
        // `cronymax-webview://<extId>/<entry>` form the renderer mounts
        // into an iframe.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.handle(webview_method::CREATE_PANEL, move |params, _ctx| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let panel_id = extract_str_field(&params, "panelId")?;
                    let title = extract_str_field(&params, "title").unwrap_or_default();
                    let entry = extract_str_field(&params, "entry")?;
                    let slot_str = extract_str_field(&params, "slot")
                        .unwrap_or_else(|_| "sidebar".to_string());
                    let slot = PanelSlot::from_idl_str(&slot_str)
                        .ok_or_else(|| ExtensionError::Rpc(format!("unknown slot `{slot_str}`")))?;
                    let view = state.webviews.create(CreatePanelArgs {
                        ext_id: ext_id.clone(),
                        panel_id: panel_id.clone(),
                        title,
                        slot,
                        entry: entry.clone(),
                    })?;
                    let url = WebviewRegistry::url_for(&ext_id, &view.panel_id, &entry);
                    Ok(Value::Map(vec![
                        (
                            Value::String("panelId".into()),
                            Value::String(view.panel_id.into()),
                        ),
                        (Value::String("url".into()), Value::String(url.into())),
                        (
                            Value::String("slot".into()),
                            Value::String(view.slot.idl_str().to_string().into()),
                        ),
                    ]))
                }
            });
        }

        // ── webview/disposePanel (notify) ─────────────────────────────
        //
        // Extension calls `panel.dispose()` → bootstrap fires this notify.
        // Idempotent against a missing id; the renderer just gets a
        // PanelDisposed event for whatever was actually present.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(webview_method::DISPOSE_PANEL, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let panel_id = extract_str_field(&params, "panelId")?;
                    let _ = state.webviews.dispose(&ext_id, &panel_id)?;
                    Ok(())
                }
            });
        }

        // ── webview/setVisible (notify) ───────────────────────────────
        //
        // Visibility toggle. The Registry validates ownership and fires
        // VisibilityChanged on the emitter so the renderer can mount /
        // unmount or show/hide the iframe.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(webview_method::SET_VISIBLE, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let panel_id = extract_str_field(&params, "panelId")?;
                    let visible = lookup_field(&params, "visible")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    state.webviews.set_visible(&ext_id, &panel_id, visible)?;
                    Ok(())
                }
            });
        }

        // ── webview/postMessage (notify) ──────────────────────────────
        //
        // Extension → iframe payload. The Registry validates ownership
        // and emits a `Message` event the renderer routes to the right
        // iframe via the cefQuery / process-message bridge. We don't
        // need to round-trip an ACK here — the IDL's `postMessage()`
        // resolves on platform receipt, which is implicit at this notify
        // boundary.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(webview_method::POST_MESSAGE, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let panel_id = extract_str_field(&params, "panelId")?;
                    let payload_rmpv = lookup_field(&params, "payload").unwrap_or(Value::Nil);
                    let payload = rmpv_to_json(&payload_rmpv);
                    state
                        .webviews
                        .deliver_message(&ext_id, &panel_id, payload)?;
                    Ok(())
                }
            });
        }

        // ── webviewView/postMessage (notify) ──────────────────────────
        //
        // Extension → operation-view iframe payload. Unlike a panel, the
        // view is not in `WebviewRegistry`; ownership is enforced against
        // the sidebar-view registry (which `registerWebviewViewProvider`
        // populates via the `sidebar/register` wire). On success we emit a
        // `Message` keyed by the viewId — the view frame self-registered
        // under that id in the renderer, so the existing C++ delivery
        // bridge routes it to the right frame.
        {
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(webview_view_method::POST_MESSAGE, move |params| {
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let view_id = extract_str_field(&params, "viewId")?;
                    let payload_rmpv = lookup_field(&params, "payload").unwrap_or(Value::Nil);
                    let payload = rmpv_to_json(&payload_rmpv);
                    let owner = state.sidebars.get(&view_id).map(|v| v.owning_ext);
                    match owner {
                        Some(o) if o == ext_id => {
                            state.webviews.deliver_to_frame(&view_id, payload);
                            Ok(())
                        }
                        Some(other) => Err(ExtensionError::BadContribution {
                            point: "cronymax.ui.sidebar.view".into(),
                            ext_id: ext_id.clone(),
                            reason: format!(
                                "view `{view_id}` is owned by `{other}`, not `{ext_id}`"
                            ),
                        }),
                        None => Err(ExtensionError::BadContribution {
                            point: "cronymax.ui.sidebar.view".into(),
                            ext_id: ext_id.clone(),
                            reason: format!(
                                "view `{view_id}` has no registered provider (did you call registerWebviewViewProvider?)"
                            ),
                        }),
                    }
                }
            });
        }

        builder.build()
    }

    // ── iframe → extension bridge ──────────────────────────────────────

    /// Forward a message that arrived from inside an iframe (via the
    /// renderer-side `acquireCronymaxApi().postMessage(payload)` →
    /// cefQuery bridge) to the owning extension's Node host as a
    /// `webview/onDidReceiveMessage` notify.
    ///
    /// Ownership is enforced at the registry level: the lookup keys the
    /// `panel_id` to its `ext_id`, so a misbehaving renderer cannot
    /// route a payload to an extension that didn't create the panel.
    pub async fn forward_panel_message(
        &self,
        panel_id: &str,
        payload: serde_json::Value,
    ) -> ExtensionResult<()> {
        // The `id` carried by the renderer can be either a webview *panel*
        // (created via `createWebviewPanel`) or an operation *view* frame
        // (contributed via `cronymax.ui.sidebar.view`). Try the panel
        // registry first; fall back to the sidebar-view registry. The two
        // route to different extension-side RPC methods so the SDK can fan
        // out to `panel.onDidReceiveMessage` vs the view provider.
        if let Some(owner) = self.state.webviews.owner_of(panel_id)? {
            let frame = Value::Map(vec![
                (
                    Value::String("panelId".into()),
                    Value::String(panel_id.to_string().into()),
                ),
                (Value::String("payload".into()), json_to_rmpv(&payload)),
            ]);
            return self
                .notify_extension(&owner, webview_method::ON_DID_RECEIVE_MESSAGE, frame)
                .await;
        }

        if let Some(view) = self.state.sidebars.get(panel_id) {
            let frame = Value::Map(vec![
                (
                    Value::String("viewId".into()),
                    Value::String(panel_id.to_string().into()),
                ),
                (Value::String("payload".into()), json_to_rmpv(&payload)),
            ]);
            return self
                .notify_extension(
                    &view.owning_ext,
                    webview_view_method::ON_DID_RECEIVE_MESSAGE,
                    frame,
                )
                .await;
        }

        Err(ExtensionError::BadContribution {
            point: "cronymax.window.panel".into(),
            ext_id: "<renderer>".into(),
            reason: format!("no panel or registered view with id `{panel_id}`"),
        })
    }

    /// Ask the extension that owns operation view `view_id` to resolve it
    /// (run its `WebviewViewProvider.resolveWebviewView`). Triggered by the
    /// web rail's `extension.view.resolve` control request right after the
    /// view's iframe is mounted.
    ///
    /// No-op (Ok) when the view has no registered provider — either the
    /// owning extension is declarative-only (no Node host) or it never
    /// called `registerWebviewViewProvider`. The view still renders; it
    /// just gets no Node-side resolve. This keeps the rail open path from
    /// erroring on views that only need to display static content.
    pub async fn resolve_view(&self, view_id: &str) -> ExtensionResult<()> {
        let Some(view) = self.state.sidebars.get(view_id) else {
            return Ok(());
        };
        // Only host-backed extensions can resolve. `notify_extension`
        // returns `NotActivated` for declarative-only ones; treat that as
        // a benign no-op rather than surfacing it to the rail.
        let frame = Value::Map(vec![
            (
                Value::String("viewId".into()),
                Value::String(view_id.to_string().into()),
            ),
            (Value::String("visible".into()), Value::Boolean(true)),
        ]);
        match self
            .notify_extension(&view.owning_ext, webview_view_method::RESOLVE, frame)
            .await
        {
            Ok(()) => Ok(()),
            Err(ExtensionError::NotActivated(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Tell the owning extension an operation view was torn down so its
    /// provider's `WebviewView.onDidDispose` fires. No-op when the view
    /// has no registered provider / live host.
    pub async fn dispose_view(&self, view_id: &str) -> ExtensionResult<()> {
        let Some(view) = self.state.sidebars.get(view_id) else {
            return Ok(());
        };
        let frame = Value::Map(vec![(
            Value::String("viewId".into()),
            Value::String(view_id.to_string().into()),
        )]);
        match self
            .notify_extension(&view.owning_ext, webview_view_method::ON_DID_DISPOSE, frame)
            .await
        {
            Ok(()) => Ok(()),
            Err(ExtensionError::NotActivated(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Tell the owning extension that operation view `view_id` became
    /// visible / hidden so its provider's `WebviewView.onDidChangeVisibility`
    /// fires and `WebviewView.visible` flips. Triggered by the web rail's
    /// `extension.view.visibility` control request when the active main /
    /// dock view changes (the view's surface stays mounted but is no longer
    /// the foreground tab, mirroring VS Code collapsing a view section).
    ///
    /// No-op (Ok) for views without a registered provider / live host. The
    /// bootstrap drops a `visible` value that matches the current state, so
    /// re-sending the same value is harmless.
    pub async fn change_view_visibility(
        &self,
        view_id: &str,
        visible: bool,
    ) -> ExtensionResult<()> {
        let Some(view) = self.state.sidebars.get(view_id) else {
            return Ok(());
        };
        let frame = Value::Map(vec![
            (
                Value::String("viewId".into()),
                Value::String(view_id.to_string().into()),
            ),
            (Value::String("visible".into()), Value::Boolean(visible)),
        ]);
        match self
            .notify_extension(
                &view.owning_ext,
                webview_view_method::ON_DID_CHANGE_VISIBILITY,
                frame,
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(ExtensionError::NotActivated(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Forward a renderer-driven view-state change (visibility / focus)
    /// to the owning extension as a `webview/onDidChangeViewState` notify.
    /// Mirrors the IDL `WebviewPanel.onDidChangeViewState` event.
    pub async fn forward_panel_view_state(
        &self,
        panel_id: &str,
        active: bool,
        visible: bool,
    ) -> ExtensionResult<()> {
        let owner = self.state.webviews.owner_of(panel_id)?.ok_or_else(|| {
            ExtensionError::BadContribution {
                point: "cronymax.window.panel".into(),
                ext_id: "<renderer>".into(),
                reason: format!("panel `{panel_id}` does not exist"),
            }
        })?;
        let frame = Value::Map(vec![
            (
                Value::String("panelId".into()),
                Value::String(panel_id.to_string().into()),
            ),
            (Value::String("active".into()), Value::Boolean(active)),
            (Value::String("visible".into()), Value::Boolean(visible)),
        ]);
        self.notify_extension(&owner, webview_method::ON_DID_CHANGE_VIEW_STATE, frame)
            .await
    }

    /// Forward a height update from a content-renderer iframe to the
    /// chat surface via the `extensions/renderer` Authority topic.
    /// Called by [`crate::runtime::handler::RuntimeHandler::
    /// handle_extension_renderer_set_height`] in response to the
    /// `kMsgRendererSetHeight` IPC the renderer-side V8 binding sends
    /// when an extension calls `setHeight(px)` inside its iframe.
    ///
    /// Currently always returns `Ok(())` — the emit is fire-and-forget;
    /// stale instance ids (extension deactivated mid-render) are caught
    /// by the chat surface dropping the event for unknown instances.
    pub async fn forward_renderer_height(&self, instance_id: &str, px: i32) -> ExtensionResult<()> {
        self.emit_renderer_event(RendererEvent::HeightChanged {
            instance_id: instance_id.to_string(),
            px,
        });
        Ok(())
    }

    /// Forward a renderer-driven panel close (user closed the iframe's
    /// tab, etc.) to the owning extension as a `webview/onDidDispose`
    /// notify and remove the panel from the registry.
    pub async fn forward_panel_disposed(&self, panel_id: &str) -> ExtensionResult<()> {
        let owner = match self.state.webviews.owner_of(panel_id)? {
            Some(o) => o,
            None => return Ok(()), // already gone
        };
        // Strip the panel under owner credentials so the dispose path
        // also fires `PanelDisposed` to the renderer (other subscribers
        // see one consistent removal).
        let _ = self.state.webviews.dispose(&owner, panel_id);
        let frame = Value::Map(vec![(
            Value::String("panelId".into()),
            Value::String(panel_id.to_string().into()),
        )]);
        self.notify_extension(&owner, webview_method::ON_DID_DISPOSE, frame)
            .await
    }
}

// ── per-EP register-handler helpers ─────────────────────────────────────

fn register_provider_handler(
    providers: &AgentProviderRegistry,
    manifest: &Manifest,
    ext_id: &str,
    params: &Value,
) -> ExtensionResult<()> {
    let provider_id = extract_str_field(params, "providerId")?;
    let decl = find_provider_decl(manifest, &provider_id).ok_or_else(|| {
        ExtensionError::BadContribution {
            point: "cronymax.agents.provider".into(),
            ext_id: ext_id.to_string(),
            reason: format!("provider `{provider_id}` not declared in manifest contributes"),
        }
    })?;
    providers.register(ProviderEntry {
        provider_id: decl.id.clone(),
        owning_ext: ext_id.to_string(),
        label: decl.label.clone(),
        icon: decl.icon.clone(),
        description: decl.description.clone(),
        supports_models: decl.supports_models.unwrap_or(false),
        supports_modes: decl.supports_modes.unwrap_or(false),
        supports_mcp: decl.supports_mcp.unwrap_or(false),
    })?;
    Ok(())
}

fn register_sidebar_handler(
    sidebars: &SidebarViewRegistry,
    manifest: &Manifest,
    ext_id: &str,
    params: &Value,
) -> ExtensionResult<()> {
    let view_id = extract_str_field(params, "viewId")?;
    let decl =
        find_sidebar_decl(manifest, &view_id).ok_or_else(|| ExtensionError::BadContribution {
            point: "cronymax.ui.sidebar.view".into(),
            ext_id: ext_id.to_string(),
            reason: format!("sidebar view `{view_id}` not declared in manifest contributes"),
        })?;
    sidebars.register(SidebarViewEntry {
        view_id: decl.id.clone(),
        owning_ext: ext_id.to_string(),
        title: decl.title.clone(),
        icon: decl.icon.clone(),
        entry: decl.entry.clone(),
    })?;
    Ok(())
}

/// Common failure handler for every L2 register notify. On failure,
/// sends `extension/registerError { ep, id, reason }` back to the
/// extension's host so the SDK shim can surface it (bootstrap.js logs
/// it via `console.error`). On success, no notify is sent.
///
/// The error is also propagated back from this fn so the RPC layer's
/// tracing logs the failure on the Rust side.
async fn report_register_outcome(
    state: &Arc<RuntimeState>,
    ext_id: &str,
    ep_id: &str,
    params: &Value,
    id_field: &str,
    result: ExtensionResult<()>,
) -> ExtensionResult<()> {
    let err = match result {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };

    let id = extract_str_field(params, id_field).unwrap_or_else(|_| "<unknown>".to_string());
    let reason = err.to_string();
    let payload = Value::Map(vec![
        (
            Value::String("ep".into()),
            Value::String(ep_id.to_string().into()),
        ),
        (Value::String("id".into()), Value::String(id.into())),
        (Value::String("reason".into()), Value::String(reason.into())),
    ]);

    let conn = state.handles.lock().get(ext_id).map(|h| h.conn.clone());
    if let Some(conn) = conn {
        if let Err(send_err) = conn.notify(method::EXTENSION_REGISTER_ERROR, payload).await {
            tracing::warn!(
                ext_id, ep_id, send_err = %send_err,
                "failed to forward register error back to extension",
            );
        }
    }
    Err(err)
}

// ── helpers ─────────────────────────────────────────────────────────────

/// Build the `events/publish` notify body. Wire shape:
/// `{ topic, publisher, data }` — mirrors `EventPayload` field-for-field
/// so the bootstrap.js dispatch can hand the JSON straight to the user
/// handler without translation.
fn build_publish_frame(_topic_for_listener: &str, payload: &EventPayload) -> Value {
    let data_rmpv = json_to_rmpv(&payload.data);
    Value::Map(vec![
        (
            Value::String("topic".into()),
            Value::String(payload.topic.clone().into()),
        ),
        (
            Value::String("publisher".into()),
            Value::String(payload.publisher.clone().into()),
        ),
        (Value::String("data".into()), data_rmpv),
    ])
}

/// Look up a field by name in a map-shaped params payload. Returns
/// `None` if the params isn't a map or the key is absent. Used by handlers
/// that want a generic-shaped value (e.g. `events/emit` `data`) instead of
/// forcing a string-only extract.
fn lookup_field(params: &Value, field: &str) -> Option<Value> {
    let map = params.as_map()?;
    for (k, v) in map {
        if k.as_str() == Some(field) {
            return Some(v.clone());
        }
    }
    None
}

/// Extract a string field from a notify params payload. Accepts either
/// `{ "field": "..." }` (the common bootstrap.js shape) or `["..."]`
/// (positional, for handwritten test RPC frames).
fn extract_str_field(params: &Value, field: &str) -> ExtensionResult<String> {
    if let Some(map) = params.as_map() {
        for (k, v) in map {
            if k.as_str() == Some(field) {
                if let Some(s) = v.as_str() {
                    return Ok(s.to_string());
                }
            }
        }
    }
    if let Some(arr) = params.as_array() {
        if let Some(first) = arr.first() {
            if let Some(s) = first.as_str() {
                return Ok(s.to_string());
            }
        }
    }
    Err(ExtensionError::Rpc(format!(
        "missing or non-string field `{field}` in notify params",
    )))
}

/// Convert an `rmpv::Value` (what the RPC layer delivers) into a
/// `serde_json::Value` so it can be re-deserialized into a typed shape
/// (e.g. [`AgentSessionEvent`]).
///
/// The conversion is total over the subset of rmpv variants any
/// JSON-source JS process can produce via `@msgpack/msgpack` (no `Ext`
/// tags, no non-string map keys). On encountering a non-string map key
/// we degrade to a stringified form rather than returning an error —
/// the dispatcher gets to see the bad shape in the resulting JSON, and
/// `serde_json::from_value` on a downstream typed deserialize will
/// report a precise field-level error.
pub(crate) fn rmpv_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Nil => J::Null,
        Value::Boolean(b) => J::Bool(*b),
        Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                serde_json::Number::from(n).into()
            } else if let Some(n) = i.as_u64() {
                serde_json::Number::from(n).into()
            } else if let Some(f) = i.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(J::Number)
                    .unwrap_or(J::Null)
            } else {
                J::Null
            }
        }
        Value::F32(f) => serde_json::Number::from_f64(*f as f64)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::F64(f) => serde_json::Number::from_f64(*f)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::String(s) => J::String(s.as_str().unwrap_or("").to_string()),
        Value::Binary(b) => J::Array(
            b.iter()
                .map(|x| serde_json::Number::from(*x as u64).into())
                .collect(),
        ),
        Value::Array(arr) => J::Array(arr.iter().map(rmpv_to_json).collect()),
        Value::Map(pairs) => {
            let mut m = serde_json::Map::with_capacity(pairs.len());
            for (k, val) in pairs {
                let key = k
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{k:?}"));
                m.insert(key, rmpv_to_json(val));
            }
            J::Object(m)
        }
        Value::Ext(tag, data) => serde_json::json!({ "__ext_tag": tag, "__ext_data": data }),
    }
}

/// Inverse of [`rmpv_to_json`] — used by dispatchers to package
/// `agents/session.create` and `agents/session.prompt` request params
/// (constructed as `serde_json::Value` for readability) into the
/// `rmpv::Value` shape the RPC connection expects on the wire. Integers
/// preserve signedness via the i64/u64 fallback ladder. Lossy on
/// `Number::is_f64() && !is_finite()` — those become `Nil`, matching the
/// rmpv_to_json behavior in the reverse direction.
pub(crate) fn json_to_rmpv(v: &serde_json::Value) -> Value {
    use serde_json::Value as J;
    match v {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i.into())
            } else if let Some(u) = n.as_u64() {
                Value::Integer(u.into())
            } else if let Some(f) = n.as_f64() {
                if f.is_finite() {
                    Value::F64(f)
                } else {
                    Value::Nil
                }
            } else {
                Value::Nil
            }
        }
        J::String(s) => Value::String(s.clone().into()),
        J::Array(arr) => Value::Array(arr.iter().map(json_to_rmpv).collect()),
        J::Object(map) => Value::Map(
            map.iter()
                .map(|(k, val)| (Value::String(k.clone().into()), json_to_rmpv(val)))
                .collect(),
        ),
    }
}

fn find_provider_decl<'a>(
    manifest: &'a Manifest,
    id: &str,
) -> Option<&'a AgentProviderContribution> {
    manifest
        .contributes
        .agent_providers
        .iter()
        .find(|p| p.id == id)
}

fn find_sidebar_decl<'a>(manifest: &'a Manifest, id: &str) -> Option<&'a SidebarViewContribution> {
    manifest
        .contributes
        .sidebar_views
        .iter()
        .find(|v| v.id == id)
}

// ── tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::rpc::{Connection, RpcServer};
    use rmpv::Value;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{duplex, split};

    fn alice_x_manifest_all_six() -> Manifest {
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
                "cronymax.agents.provider": [
                    {
                        "id": "alice.x.gpt",
                        "label": "Alice GPT",
                        "supportsModels": true
                    }
                ],
                "cronymax.content.renderer": [
                    { "id": "alice.x.rend", "mimeTypes": ["text/x-alice"], "entry": "./r.html" }
                ],
                "cronymax.ui.sidebar.view": [
                    { "id": "alice.x.view", "title": "Alice View", "entry": "./v.html" }
                ]
            }
        }"#;
        Manifest::from_json(raw).unwrap()
    }

    /// Build a runtime wired to a duplex pair: one side runs the
    /// runtime's RPC handlers, the other side is the "extension" that
    /// drives notify traffic.
    ///
    /// The runtime's `state.handles[alice.x]` is populated with the
    /// runtime-side conn, mimicking what `activate()` does after spawn.
    /// Returns the runtime and the peer conn the test uses.
    async fn wired_pair() -> (ExtensionRuntime, Arc<Connection>) {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", manifest.clone());

        // Mirror the activate-time manifest-driven population of the
        // content-renderer registry (real `activate()` does this from
        // `ingest_manifest`; the test side-steps full activate so we
        // replicate the step here so renderer tests see a populated
        // registry).
        runtime
            .state
            .renderers
            .ingest_manifest(&manifest, "alice.x")
            .unwrap();

        let rpc_for_runtime_side = runtime.build_per_extension_handlers("alice.x", &manifest);

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, rpc_for_runtime_side);
        let (peer_conn, _t2) = Connection::open(b_r, b_w, RpcServer::builder().build());

        // We need an ExtensionHandle to live in state.handles so the
        // register handlers can find the conn (and so registerError
        // notifies can flow back). But ExtensionHandle requires a real
        // NodeHost. For unit testing we side-step by inserting only the
        // conn through a test-only handle accessor below.
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);

        (runtime, peer_conn)
    }

    type CapturedErrors = Arc<StdMutex<Vec<(String, String, String)>>>;

    /// Capture the contents of `extension/registerError` notifies sent
    /// from the runtime back to the extension. Attached to the peer's
    /// RPC server.
    fn capture_register_errors() -> (CapturedErrors, RpcServer) {
        let captured: CapturedErrors = Arc::new(StdMutex::new(Vec::new()));
        let c = captured.clone();
        let server = RpcServer::builder()
            .on_notify(method::EXTENSION_REGISTER_ERROR, move |params| {
                let c = c.clone();
                async move {
                    let ep = extract_str_field(&params, "ep").unwrap_or_default();
                    let id = extract_str_field(&params, "id").unwrap_or_default();
                    let reason = extract_str_field(&params, "reason").unwrap_or_default();
                    c.lock().unwrap().push((ep, id, reason));
                    Ok(())
                }
            })
            .build();
        (captured, server)
    }

    /// Like `wired_pair`, but the peer side installs a `registerError`
    /// notify capturer so tests can assert on the error path.
    async fn wired_pair_with_error_capture() -> (ExtensionRuntime, Arc<Connection>, CapturedErrors)
    {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", manifest.clone());
        let rpc_for_runtime_side = runtime.build_per_extension_handlers("alice.x", &manifest);
        let (captured, peer_server) = capture_register_errors();

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, rpc_for_runtime_side);
        let (peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);

        runtime.install_test_handle_conn_only("alice.x", runtime_conn);
        (runtime, peer_conn, captured)
    }

    // Adds a NodeHost-less handle to state.handles so unit tests can
    // exercise the register-notify path without a real subprocess.
    // Thin alias kept for the existing call sites in this module;
    // delegates to the crate-visible `install_test_handle`.
    impl ExtensionRuntime {
        fn install_test_handle_conn_only(&self, ext_id: &str, conn: Arc<Connection>) {
            self.install_test_handle(ext_id, conn);
        }
    }

    #[tokio::test]
    async fn extension_register_provider_lands_in_registry() {
        let (runtime, peer_conn) = wired_pair().await;
        peer_conn
            .notify(
                agents_method::REGISTER_PROVIDER,
                Value::Map(vec![(
                    Value::String("providerId".into()),
                    Value::String("alice.x.gpt".into()),
                )]),
            )
            .await
            .unwrap();
        for _ in 0..50 {
            if runtime.providers().len() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let p = runtime.providers().get("alice.x.gpt").expect("registered");
        assert_eq!(p.owning_ext, "alice.x");
        assert_eq!(p.label, "Alice GPT");
        assert!(p.supports_models);
    }

    #[tokio::test]
    async fn extension_register_then_unregister_provider() {
        let (runtime, peer) = wired_pair().await;
        peer.notify(
            agents_method::REGISTER_PROVIDER,
            Value::Map(vec![(
                Value::String("providerId".into()),
                Value::String("alice.x.gpt".into()),
            )]),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if runtime.providers().len() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(runtime.providers().len(), 1);

        peer.notify(
            agents_method::UNREGISTER_PROVIDER,
            Value::Map(vec![(
                Value::String("providerId".into()),
                Value::String("alice.x.gpt".into()),
            )]),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if runtime.providers().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(runtime.providers().is_empty());
    }

    #[tokio::test]
    async fn extension_register_undeclared_provider_emits_register_error() {
        let (runtime, peer, captured) = wired_pair_with_error_capture().await;
        peer.notify(
            agents_method::REGISTER_PROVIDER,
            Value::Map(vec![(
                Value::String("providerId".into()),
                Value::String("alice.x.NOTDECLARED".into()),
            )]),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if !captured.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let errs = captured.lock().unwrap().clone();
        assert_eq!(errs.len(), 1, "expected one registerError notify");
        let (ep, id, reason) = &errs[0];
        assert_eq!(ep, "cronymax.agents.provider");
        assert_eq!(id, "alice.x.NOTDECLARED");
        assert!(
            reason.contains("not declared in manifest"),
            "reason did not contain the manifest-not-declared message: {reason:?}",
        );
        // Registry stayed empty.
        assert!(runtime.providers().is_empty());
    }

    #[tokio::test]
    async fn extension_command_register_unregister_round_trip() {
        let (runtime, peer) = wired_pair().await;
        peer.notify(
            method::COMMANDS_REGISTER,
            Value::Map(vec![(
                Value::String("commandId".into()),
                Value::String("alice.x.hi".into()),
            )]),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if runtime.state.commands.lock().len() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            runtime.state.commands.lock().owner_of("alice.x.hi"),
            Some("alice.x"),
        );

        peer.notify(
            method::COMMANDS_UNREGISTER,
            Value::Map(vec![(
                Value::String("commandId".into()),
                Value::String("alice.x.hi".into()),
            )]),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if runtime.state.commands.lock().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(runtime.state.commands.lock().is_empty());
    }

    #[tokio::test]
    async fn manifest_renderer_contribution_lands_in_registry() {
        // P6.5 — content renderers are manifest-driven (no Node-side
        // register RPC). `wired_pair` mirrors the activate-time call to
        // `ContentRendererRegistry::ingest_manifest`, so the registry must
        // already be populated by the time the peer connection is up.
        let (runtime, _peer) = wired_pair().await;
        let r = runtime.renderers().get("alice.x.rend").unwrap();
        assert_eq!(r.owning_ext, "alice.x");
        assert_eq!(r.mime_types, vec!["text/x-alice".to_string()]);
        assert_eq!(
            runtime
                .renderers()
                .first_for_mime("text/x-alice")
                .unwrap()
                .renderer_id,
            "alice.x.rend"
        );
    }

    #[tokio::test]
    async fn extension_sidebar_register_lands_in_registry() {
        let (runtime, peer) = wired_pair().await;
        peer.notify(
            sidebar_method::REGISTER,
            Value::Map(vec![(
                Value::String("viewId".into()),
                Value::String("alice.x.view".into()),
            )]),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if runtime.sidebars().len() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let v = runtime.sidebars().get("alice.x.view").unwrap();
        assert_eq!(v.title, "Alice View");
        assert_eq!(v.owning_ext, "alice.x");
    }

    #[tokio::test]
    async fn activate_unknown_extension_returns_not_installed() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let err = runtime
            .activate("ghost.ext", |_, _| panic!("never called"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ExtensionError::NotInstalled(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn activate_disabled_extension_returns_not_enabled() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test_with_enabled("alice.x", manifest, false);
        let err = runtime
            .activate("alice.x", |_, _| panic!("never called"))
            .await
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NotEnabled(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn deactivate_unknown_extension_returns_not_activated() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let err = runtime.deactivate("nope").await.unwrap_err();
        assert!(
            matches!(err, ExtensionError::NotActivated(_)),
            "got {err:?}"
        );
    }

    /// Declarative-only extension (no `manifest.main`) activates and
    /// deactivates without spawning any Node host. Contributions ingested
    /// during activate are visible immediately; `state.handles` stays
    /// empty; deactivate cleans the typed registries and clears the
    /// lifecycle bit.
    #[tokio::test]
    async fn activate_declarative_only_extension_skips_node_host() {
        let raw = r#"{
            "id": "acme.statics",
            "name": "Statics",
            "version": "0.1.0",
            "publisher": "acme",
            "engines": { "cronymax": "^1.0" },
            "activationEvents": [],
            "contributes": {
                "cronymax.content.renderer": [
                    { "id": "acme.statics.r", "mimeTypes": ["text/x-foo"], "entry": "./r.html" }
                ]
            }
        }"#;
        let manifest = Manifest::from_json(raw).expect("valid manifest");
        assert!(manifest.main.is_none(), "main must be unset");

        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("acme.statics", manifest);

        // The cfg_builder would only be called for host-backed
        // extensions, so panic if it fires — proves we didn't take the
        // spawn path.
        runtime
            .activate("acme.statics", |_, _| {
                panic!("cfg_builder must not run for declarative-only extension")
            })
            .await
            .expect("activate should succeed without spawning a host");

        // Renderer registry populated from manifest.
        assert_eq!(runtime.renderers().len(), 1);
        let r = runtime.renderers().get("acme.statics.r").unwrap();
        assert_eq!(r.owning_ext, "acme.statics");

        // No live host handle.
        assert!(
            runtime.state.handles.lock().get("acme.statics").is_none(),
            "declarative-only extension must not insert a host handle",
        );

        // Lifecycle records the activation.
        assert!(runtime.state.lifecycle.lock().is_activated("acme.statics"));

        // Deactivate cleans up.
        runtime
            .deactivate("acme.statics")
            .await
            .expect("deactivate should succeed without a host");
        assert_eq!(runtime.renderers().len(), 0);
        assert!(!runtime.state.lifecycle.lock().is_activated("acme.statics"));
    }

    /// P6.5-T05: `forward_renderer_height` fires a `RendererEvent::
    /// HeightChanged` through the composition-root-installed emitter.
    /// The chat surface subscribes to the `extensions/renderer` topic
    /// the emitter routes into via `RuntimeAuthority::emit`.
    /// P6.5-T10 dogfood: load the real `examples/mermaid-renderer/
    /// cronymax-extension.json` from disk, validate it through the same
    /// `Manifest::from_json` path the registry uses at install time, and
    /// confirm that an `ExtensionRuntime` activation populates the
    /// content-renderer registry as expected. This proves the dogfood
    /// fixture stays in sync with whatever manifest schema changes we
    /// make — if anyone breaks the manifest contract, this test fires.
    #[tokio::test]
    async fn dogfood_mermaid_renderer_fixture_activates_cleanly() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace parent")
            .parent()
            .expect("workspace root");
        let manifest_path = repo_root
            .join("examples")
            .join("mermaid-renderer")
            .join("cronymax-extension.json");
        let raw = std::fs::read_to_string(&manifest_path)
            .expect("read examples/mermaid-renderer/cronymax-extension.json");
        let manifest = Manifest::from_json(&raw).expect("manifest parses");
        // The dogfood example MUST be declarative-only (no Node host) so
        // the renderer-only activation path it exercises stays load-bearing.
        assert!(
            manifest.main.is_none(),
            "mermaid-renderer fixture must remain declarative-only",
        );
        assert_eq!(manifest.id, "cronymax-examples.mermaid-renderer");

        let ext_id = manifest.id.clone();
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        runtime
            .state
            .registry
            .lock()
            .insert_for_test(&ext_id, manifest);
        runtime
            .activate(&ext_id, |_, _| {
                panic!("cfg_builder must not run for declarative-only extension")
            })
            .await
            .expect("activate fixture");

        // Renderer registry must now hold the mermaid renderer keyed by
        // its mime type so the chat dispatcher can find it.
        let r = runtime
            .renderers()
            .first_for_mime("text/vnd.mermaid")
            .expect("text/vnd.mermaid renderer registered");
        assert_eq!(r.owning_ext, "cronymax-examples.mermaid-renderer");
        assert!(r.entry.ends_with("renderer/index.html"));
    }

    /// Dogfood: load the real `examples/panel-explorer/cronymax-extension.json`
    /// and confirm activation ingests both operation-view contributions
    /// (one `target: main`, one `target: right`) into the contribution
    /// registry the activity-bar rail reads from. Keeps the fixture in sync
    /// with the manifest schema (`target` field, declarative-only).
    #[tokio::test]
    async fn dogfood_panel_explorer_fixture_activates_cleanly() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace parent")
            .parent()
            .expect("workspace root");
        let manifest_path = repo_root
            .join("examples")
            .join("panel-explorer")
            .join("cronymax-extension.json");
        let raw = std::fs::read_to_string(&manifest_path)
            .expect("read examples/panel-explorer/cronymax-extension.json");
        let manifest = Manifest::from_json(&raw).expect("manifest parses");
        assert!(
            manifest.main.is_none(),
            "panel-explorer fixture must remain declarative-only",
        );
        assert_eq!(manifest.id, "cronymax-examples.panel-explorer");

        let ext_id = manifest.id.clone();
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        runtime
            .state
            .registry
            .lock()
            .insert_for_test(&ext_id, manifest);
        runtime
            .activate(&ext_id, |_, _| {
                panic!("cfg_builder must not run for declarative-only extension")
            })
            .await
            .expect("activate fixture");

        use crate::extensions::contributions::kind::UI_SIDEBAR_VIEW;
        let views: Vec<_> = runtime
            .contributions_snapshot()
            .into_iter()
            .filter(|d| d.kind == UI_SIDEBAR_VIEW)
            .collect();
        assert_eq!(views.len(), 2, "two operation views declared");

        let main = views
            .iter()
            .find(|d| d.id == "cronymax-examples.panel-explorer.main")
            .expect("main view present");
        assert_eq!(main.metadata["target"], "main");
        assert_eq!(main.metadata["entry"], "view/index.html");

        let dock = views
            .iter()
            .find(|d| d.id == "cronymax-examples.panel-explorer.dock")
            .expect("dock view present");
        assert_eq!(dock.metadata["target"], "right");
    }

    /// End-to-end of the *real* startup path the app runs: drop the example
    /// into a registry root, `refresh()` (which validates the manifest — the
    /// step `dogfood_panel_explorer_fixture_activates_cleanly` skips), then
    /// activate and confirm the operation views reach the contribution list
    /// the rail reads. Guards against a manifest that parses but fails
    /// validation (and so would silently never appear in the rail).
    #[tokio::test]
    async fn panel_explorer_install_refresh_activate_surfaces_views() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let manifest_raw = std::fs::read_to_string(
            repo_root
                .join("examples")
                .join("panel-explorer")
                .join("cronymax-extension.json"),
        )
        .expect("read example manifest");

        // Lay the example out exactly like an install: <root>/<id>/manifest.
        let root = tempfile::TempDir::new().unwrap();
        let ext_dir = root.path().join("cronymax-examples.panel-explorer");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("cronymax-extension.json"), &manifest_raw).unwrap();

        // refresh() scans + validates + reconciles registry.json.
        let mut registry = ExtensionRegistry::new(root.path().to_path_buf());
        registry.refresh().expect("refresh");
        let entry = registry
            .get("cronymax-examples.panel-explorer")
            .expect("entry present after refresh");
        assert!(entry.enabled, "out-of-band install defaults enabled");

        // Activate (declarative — no Node host) and confirm the views land.
        let runtime = ExtensionRuntime::new(registry);
        runtime
            .activate("cronymax-examples.panel-explorer", |_, _| {
                panic!("declarative-only")
            })
            .await
            .expect("activate");

        use crate::extensions::contributions::kind::UI_SIDEBAR_VIEW;
        let views = runtime
            .contributions_snapshot()
            .into_iter()
            .filter(|d| d.kind == UI_SIDEBAR_VIEW)
            .count();
        assert_eq!(views, 2, "both operation views reach the contribution list");
    }

    #[tokio::test]
    async fn activate_and_deactivate_fire_contributions_changed() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let count: Arc<StdMutex<usize>> = Arc::new(StdMutex::new(0));
        let c = count.clone();
        runtime.set_contributions_emitter(Arc::new(move || {
            *c.lock().unwrap() += 1;
        }));

        // Declarative manifest (no `main`) so activate stays in-process.
        let manifest = Manifest::from_json(
            r#"{
                "id": "alice.x",
                "name": "X",
                "version": "0.1.0",
                "publisher": "alice",
                "engines": { "cronymax": "^1.0" },
                "activationEvents": [],
                "contributes": {
                    "cronymax.ui.sidebar.view": [
                        { "id": "alice.x.v", "title": "V", "entry": "v.html" }
                    ]
                }
            }"#,
        )
        .unwrap();
        let ext_id = manifest.id.clone();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test(&ext_id, manifest);

        runtime
            .activate(&ext_id, |_, _| panic!("declarative-only"))
            .await
            .unwrap();
        let after_activate = *count.lock().unwrap();
        assert!(after_activate >= 1, "activate fires contributions-changed");

        runtime.deactivate(&ext_id).await.unwrap();
        assert!(
            *count.lock().unwrap() > after_activate,
            "deactivate fires contributions-changed"
        );
    }

    #[tokio::test]
    async fn forward_renderer_height_emits_height_changed_event() {
        use crate::extensions::api::renderers::RendererEvent;
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let captured: Arc<StdMutex<Vec<RendererEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let cap = captured.clone();
        runtime.set_renderer_emitter(Arc::new(move |ev| {
            cap.lock().unwrap().push(ev);
        }));

        runtime
            .forward_renderer_height("inst-42", 192)
            .await
            .expect("forward_renderer_height should succeed");

        let events = captured.lock().unwrap().clone();
        assert!(matches!(
            events.as_slice(),
            [RendererEvent::HeightChanged { instance_id, px }]
                if instance_id == "inst-42" && *px == 192
        ));
    }

    #[tokio::test]
    async fn send_to_extension_unknown_returns_not_activated() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let err = runtime
            .send_to_extension("ghost", "any", Value::Nil)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ExtensionError::NotActivated(_)),
            "got {err:?}",
        );
    }

    #[tokio::test]
    async fn notify_extension_unknown_returns_not_activated() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let err = runtime
            .notify_extension("ghost", "any", Value::Nil)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ExtensionError::NotActivated(_)),
            "got {err:?}",
        );
    }

    #[tokio::test]
    async fn send_to_extension_routes_via_handle_conn() {
        // Set up the same duplex as wired_pair, but on the peer side
        // install an echo handler so we can verify send_to_extension
        // actually reaches it.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", manifest.clone());

        let echo_server = RpcServer::builder()
            .handle("echo", |params, _| async move { Ok(params) })
            .build();
        let runtime_server = runtime.build_per_extension_handlers("alice.x", &manifest);
        let (a, b) = duplex(4096);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, runtime_server);
        let (_peer_conn, _t2) = Connection::open(b_r, b_w, echo_server);
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);

        let resp = runtime
            .send_to_extension(
                "alice.x",
                "echo",
                Value::Array(vec![Value::String("hi".into())]),
            )
            .await
            .unwrap();
        match resp {
            Value::Array(items) => assert_eq!(
                items
                    .first()
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                Some("hi".to_string()),
            ),
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn json_to_rmpv_and_back_round_trips_dispatcher_params() {
        let original = serde_json::json!({
            "sessionId": "s-1",
            "message": {
                "text": "hello",
                "attachments": []
            },
            "model": null,
            "n": 42,
        });
        let rmpv = json_to_rmpv(&original);
        let back = rmpv_to_json(&rmpv);
        assert_eq!(back, original);
    }

    #[tokio::test]
    async fn contributions_for_ep_returns_ingested_values() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime.state.contributions.lock().ingest(&manifest);
        // Each command now becomes its own descriptor; the metadata of
        // that descriptor is the command's manifest object (minus `id`).
        let cmds = runtime.contributions_for_ep("cronymax.command");
        assert_eq!(cmds.len(), 1);
        // Round-trip through the descriptor lookup to confirm the id key
        // survives ingestion as the descriptor id.
        let d = runtime
            .contribution_get("cronymax.command", "alice.x.hi")
            .expect("descriptor present");
        assert_eq!(d.label, "Hi");
    }

    #[tokio::test]
    async fn agents_event_notify_routes_to_registered_dispatcher_sink() {
        let (runtime, peer_conn) = wired_pair().await;

        let session_id = "s-evt".to_string();
        let mut rx = runtime
            .session_router()
            .register(session_id.clone())
            .unwrap();

        // Peer side emits an `agents/event` notify with a Text payload
        // shaped exactly like the bootstrap.js wire format.
        peer_conn
            .notify(
                agents_method::EVENT,
                Value::Map(vec![
                    (
                        Value::String("sessionId".into()),
                        Value::String(session_id.clone().into()),
                    ),
                    (
                        Value::String("event".into()),
                        Value::Map(vec![
                            (Value::String("kind".into()), Value::String("text".into())),
                            (
                                Value::String("text".into()),
                                Value::String("hello chat".into()),
                            ),
                        ]),
                    ),
                ]),
            )
            .await
            .unwrap();

        // The dispatcher sink should observe the typed event.
        let msg = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("dispatcher should receive event within 500ms")
            .expect("channel should still be open");
        match msg {
            crate::extensions::api::agents::AgentSessionMessage::Event(
                AgentSessionEvent::Text { text },
            ) => assert_eq!(text, "hello chat"),
            other => panic!("expected Event(Text), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn agents_turn_done_notify_routes_to_dispatcher_sink() {
        let (runtime, peer_conn) = wired_pair().await;

        let session_id = "s-end".to_string();
        let mut rx = runtime
            .session_router()
            .register(session_id.clone())
            .unwrap();

        peer_conn
            .notify(
                agents_method::TURN_DONE,
                Value::Map(vec![(
                    Value::String("sessionId".into()),
                    Value::String(session_id.clone().into()),
                )]),
            )
            .await
            .unwrap();

        let msg = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("dispatcher should receive turn.done within 500ms")
            .expect("channel should still be open");
        assert!(matches!(
            msg,
            crate::extensions::api::agents::AgentSessionMessage::TurnDone
        ));
    }

    #[tokio::test]
    async fn agents_event_notify_for_unregistered_session_is_dropped_quietly() {
        let (_runtime, peer_conn) = wired_pair().await;
        // No router registration for "ghost-session". The runtime should
        // accept the notify, log a debug line, and not crash. We assert
        // by sending a follow-up `$/ping` request that succeeds.
        peer_conn
            .notify(
                agents_method::EVENT,
                Value::Map(vec![
                    (
                        Value::String("sessionId".into()),
                        Value::String("ghost-session".into()),
                    ),
                    (
                        Value::String("event".into()),
                        Value::Map(vec![
                            (Value::String("kind".into()), Value::String("text".into())),
                            (
                                Value::String("text".into()),
                                Value::String("orphaned".into()),
                            ),
                        ]),
                    ),
                ]),
            )
            .await
            .unwrap();

        // Yield so the dispatch task processes the notify before we tear
        // down the duplex — we don't care about a reply, just that nothing
        // panicked.
        tokio::task::yield_now().await;
    }

    // ── P5 · platform-event bus integration ────────────────────────────

    /// Build a wired pair where the runtime side has already had alice.x
    /// registered with the event bus carrying the supplied subscribe /
    /// emit caps. The peer side installs a capture for `events/publish`
    /// so tests can assert on platform → extension fan-out.
    async fn wired_pair_with_event_capture(
        subscribe: &[&str],
        emit: &[&str],
    ) -> (
        ExtensionRuntime,
        Arc<Connection>,
        Arc<StdMutex<Vec<(String, String, serde_json::Value)>>>,
    ) {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", manifest.clone());
        runtime
            .state
            .events
            .register_extension(
                "alice.x",
                &subscribe.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                &emit.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
            .expect("register_extension on a fresh bus must succeed");

        let rpc_for_runtime_side = runtime.build_per_extension_handlers("alice.x", &manifest);

        // Capture incoming events/publish on the peer side.
        type Captured = Arc<StdMutex<Vec<(String, String, serde_json::Value)>>>;
        let captured: Captured = Arc::new(StdMutex::new(Vec::new()));
        let cap_c = captured.clone();
        let peer_server = RpcServer::builder()
            .on_notify(method::EVENTS_PUBLISH, move |params| {
                let cap = cap_c.clone();
                async move {
                    let topic = extract_str_field(&params, "topic").unwrap_or_default();
                    let publisher = extract_str_field(&params, "publisher").unwrap_or_default();
                    let data = lookup_field(&params, "data")
                        .map(|v| rmpv_to_json(&v))
                        .unwrap_or(serde_json::Value::Null);
                    cap.lock().unwrap().push((topic, publisher, data));
                    Ok(())
                }
            })
            .build();

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, rpc_for_runtime_side);
        let (peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);
        (runtime, peer_conn, captured)
    }

    async fn wait_until<F: Fn() -> bool>(pred: F) {
        for _ in 0..100 {
            if pred() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn platform_emit_reaches_subscribed_extension_via_publish_notify() {
        let (runtime, peer_conn, captured) =
            wired_pair_with_event_capture(&["cronymax.session.started"], &[]).await;

        // Extension declares its intent to receive the topic.
        peer_conn
            .notify(
                method::EVENTS_SUBSCRIBE,
                Value::Map(vec![(
                    Value::String("topic".into()),
                    Value::String("cronymax.session.started".into()),
                )]),
            )
            .await
            .unwrap();

        wait_until(|| {
            runtime
                .events()
                .subscriber_count("cronymax.session.started")
                == 1
        })
        .await;
        assert_eq!(
            runtime
                .events()
                .subscriber_count("cronymax.session.started"),
            1,
            "events/subscribe notify must install exactly one bus listener",
        );

        // Platform emits — must arrive over events/publish.
        runtime.events().emit_from_platform(
            PlatformTopic::SessionStarted,
            serde_json::json!({ "sessionId": "s-1", "providerId": "coco" }),
        );

        wait_until(|| !captured.lock().unwrap().is_empty()).await;
        let caps = captured.lock().unwrap().clone();
        assert_eq!(caps.len(), 1, "exactly one events/publish notify");
        let (topic, publisher, data) = &caps[0];
        assert_eq!(topic, "cronymax.session.started");
        assert_eq!(publisher, "cronymax");
        assert_eq!(
            data.get("providerId").and_then(|v| v.as_str()),
            Some("coco"),
        );
    }

    #[tokio::test]
    async fn subscribe_without_capability_does_not_install_listener() {
        // Extension declared NO subscribe caps; events/subscribe should
        // fail capability gating, the bus stays at zero subscribers, and
        // the platform emit drops on the floor.
        let (runtime, peer_conn, captured) = wired_pair_with_event_capture(&[], &[]).await;
        peer_conn
            .notify(
                method::EVENTS_SUBSCRIBE,
                Value::Map(vec![(
                    Value::String("topic".into()),
                    Value::String("cronymax.session.started".into()),
                )]),
            )
            .await
            .unwrap();

        // Give the runtime time to process the notify (which should error).
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            runtime
                .events()
                .subscriber_count("cronymax.session.started"),
            0,
            "capability-denied subscribe must not install a listener",
        );

        runtime
            .events()
            .emit_from_platform(PlatformTopic::SessionStarted, serde_json::json!({}));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(
            captured.lock().unwrap().is_empty(),
            "extension without subscribe cap must not receive events/publish",
        );
    }

    #[tokio::test]
    async fn unsubscribe_severs_the_forwarding_listener() {
        let (runtime, peer_conn, captured) =
            wired_pair_with_event_capture(&["cronymax.tool.invoked"], &[]).await;

        peer_conn
            .notify(
                method::EVENTS_SUBSCRIBE,
                Value::Map(vec![(
                    Value::String("topic".into()),
                    Value::String("cronymax.tool.invoked".into()),
                )]),
            )
            .await
            .unwrap();
        wait_until(|| runtime.events().subscriber_count("cronymax.tool.invoked") == 1).await;

        peer_conn
            .notify(
                method::EVENTS_UNSUBSCRIBE,
                Value::Map(vec![(
                    Value::String("topic".into()),
                    Value::String("cronymax.tool.invoked".into()),
                )]),
            )
            .await
            .unwrap();
        wait_until(|| runtime.events().subscriber_count("cronymax.tool.invoked") == 0).await;

        runtime.events().emit_from_platform(
            PlatformTopic::ToolInvoked,
            serde_json::json!({ "name": "shell" }),
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(
            captured.lock().unwrap().is_empty(),
            "unsubscribed listener must not receive forwarding notifies",
        );
    }

    #[tokio::test]
    async fn extension_emit_request_succeeds_for_declared_topic() {
        let (runtime, peer_conn, _captured) =
            wired_pair_with_event_capture(&[], &["alice.x.heartbeat"]).await;
        let _ = peer_conn
            .request(
                method::EVENTS_EMIT,
                Value::Map(vec![
                    (
                        Value::String("topic".into()),
                        Value::String("alice.x.heartbeat".into()),
                    ),
                    (
                        Value::String("data".into()),
                        Value::Map(vec![(
                            Value::String("seq".into()),
                            Value::Integer(7i64.into()),
                        )]),
                    ),
                ]),
            )
            .await
            .expect("emit on a declared topic must succeed");
        // The bus has no subscribers, but the request itself must round-trip.
        // Smoke-check: subscriber_count for the topic is still zero.
        assert_eq!(runtime.events().subscriber_count("alice.x.heartbeat"), 0);
    }

    #[tokio::test]
    async fn extension_emit_request_rejects_undeclared_topic() {
        let (_runtime, peer_conn, _captured) = wired_pair_with_event_capture(&[], &[]).await;
        let err = peer_conn
            .request(
                method::EVENTS_EMIT,
                Value::Map(vec![(
                    Value::String("topic".into()),
                    Value::String("alice.x.unauthorized".into()),
                )]),
            )
            .await
            .expect_err("undeclared emit must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("did not declare events.emit") || msg.contains("CapabilityDenied"),
            "expected capability-denied error, got `{msg}`",
        );
    }

    #[tokio::test]
    async fn extension_emit_request_rejects_cronymax_topic() {
        let (_runtime, peer_conn, _captured) = wired_pair_with_event_capture(&[], &["*"]).await;
        let err = peer_conn
            .request(
                method::EVENTS_EMIT,
                Value::Map(vec![(
                    Value::String("topic".into()),
                    Value::String("cronymax.message.user.sent".into()),
                )]),
            )
            .await
            .expect_err("emit on a reserved cronymax.* topic must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("reserved") || msg.contains("NamespaceReserved"),
            "expected namespace-reserved error, got `{msg}`",
        );
    }

    #[tokio::test]
    async fn cross_extension_emit_reaches_other_subscribers() {
        // Two extensions, both wired through the same EventBus instance:
        // alice.x emits `alice.x.beat`, bob.y subscribes to it.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let alice_manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", alice_manifest.clone());
        runtime
            .state
            .events
            .register_extension("alice.x", &[], &["alice.x.*".to_string()])
            .unwrap();
        runtime
            .state
            .events
            .register_extension("bob.y", &["alice.x.beat".to_string()], &[])
            .unwrap();

        // Wire alice.x with its own RPC handler + a peer with no capture
        // (we don't need to assert on alice).
        let alice_rpc = runtime.build_per_extension_handlers("alice.x", &alice_manifest);
        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (alice_conn, _t1) = Connection::open(a_r, a_w, alice_rpc);
        let (alice_peer, _t2) = Connection::open(b_r, b_w, RpcServer::builder().build());
        runtime.install_test_handle_conn_only("alice.x", alice_conn);

        // Wire bob.y with a capture so we can see the forwarded notify.
        let bob_manifest = Manifest::from_json(
            r#"{
                "id": "bob.y", "name": "Y", "version": "0.1.0",
                "publisher": "bob",
                "engines": { "cronymax": "^1.0" },
                "main": "./m.js",
                "activationEvents": [],
                "contributes": {}
            }"#,
        )
        .unwrap();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("bob.y", bob_manifest.clone());
        let bob_rpc = runtime.build_per_extension_handlers("bob.y", &bob_manifest);
        type Captured = Arc<StdMutex<Vec<(String, String)>>>;
        let captured: Captured = Arc::new(StdMutex::new(Vec::new()));
        let cap_c = captured.clone();
        let bob_peer_server = RpcServer::builder()
            .on_notify(method::EVENTS_PUBLISH, move |params| {
                let cap = cap_c.clone();
                async move {
                    let topic = extract_str_field(&params, "topic").unwrap_or_default();
                    let publisher = extract_str_field(&params, "publisher").unwrap_or_default();
                    cap.lock().unwrap().push((topic, publisher));
                    Ok(())
                }
            })
            .build();
        let (c, d) = duplex(8192);
        let (c_r, c_w) = split(c);
        let (d_r, d_w) = split(d);
        let (bob_conn, _t3) = Connection::open(c_r, c_w, bob_rpc);
        let (bob_peer, _t4) = Connection::open(d_r, d_w, bob_peer_server);
        runtime.install_test_handle_conn_only("bob.y", bob_conn);

        // Bob subscribes to alice.x.beat.
        bob_peer
            .notify(
                method::EVENTS_SUBSCRIBE,
                Value::Map(vec![(
                    Value::String("topic".into()),
                    Value::String("alice.x.beat".into()),
                )]),
            )
            .await
            .unwrap();
        wait_until(|| runtime.events().subscriber_count("alice.x.beat") == 1).await;

        // Alice emits.
        alice_peer
            .request(
                method::EVENTS_EMIT,
                Value::Map(vec![
                    (
                        Value::String("topic".into()),
                        Value::String("alice.x.beat".into()),
                    ),
                    (
                        Value::String("data".into()),
                        Value::Map(vec![(
                            Value::String("n".into()),
                            Value::Integer(42i64.into()),
                        )]),
                    ),
                ]),
            )
            .await
            .expect("alice's emit must succeed");

        // Bob's peer should receive an events/publish whose publisher is
        // alice.x (not cronymax).
        wait_until(|| !captured.lock().unwrap().is_empty()).await;
        let caps = captured.lock().unwrap().clone();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].0, "alice.x.beat");
        assert_eq!(caps[0].1, "alice.x");
    }

    // ── P6 · webview panel RPC integration ─────────────────────────────

    /// Build a wired pair where the runtime has alice.x's manifest
    /// installed and an emitter that captures every WebviewEvent. The
    /// peer side has no extra handlers (tests drive notifies and
    /// requests directly).
    async fn wired_pair_with_webview_capture() -> (
        ExtensionRuntime,
        Arc<Connection>,
        Arc<StdMutex<Vec<crate::extensions::api::webview::WebviewEvent>>>,
    ) {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", manifest.clone());

        let log: Arc<StdMutex<Vec<crate::extensions::api::webview::WebviewEvent>>> =
            Arc::new(StdMutex::new(Vec::new()));
        let log_c = log.clone();
        runtime.set_webview_emitter(Arc::new(move |ev| {
            log_c.lock().unwrap().push(ev);
        }));

        let rpc_for_runtime_side = runtime.build_per_extension_handlers("alice.x", &manifest);
        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, rpc_for_runtime_side);
        let (peer_conn, _t2) = Connection::open(b_r, b_w, RpcServer::builder().build());
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);
        (runtime, peer_conn, log)
    }

    #[tokio::test]
    async fn webview_create_panel_request_returns_url_and_fires_event() {
        let (runtime, peer_conn, log) = wired_pair_with_webview_capture().await;
        let resp = peer_conn
            .request(
                webview_method::CREATE_PANEL,
                Value::Map(vec![
                    (
                        Value::String("panelId".into()),
                        Value::String("p-hello".into()),
                    ),
                    (Value::String("title".into()), Value::String("Hello".into())),
                    (
                        Value::String("slot".into()),
                        Value::String("sidebar".into()),
                    ),
                    (
                        Value::String("entry".into()),
                        Value::String("./hello.html".into()),
                    ),
                ]),
            )
            .await
            .expect("createPanel request must succeed");
        let url = rmpv_to_json(&resp)
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        assert_eq!(
            url.as_deref(),
            Some("cronymax-webview://alice.x/hello.html?surface=panel&id=p-hello"),
        );
        assert_eq!(runtime.webviews().len(), 1);
        let events = log.lock().unwrap().clone();
        assert!(
            matches!(
                &events[..],
                [crate::extensions::api::webview::WebviewEvent::PanelCreated {
                    panel_id, slot, url,
                    ..
                }] if panel_id == "p-hello" && slot == "sidebar"
                    && url == "cronymax-webview://alice.x/hello.html?surface=panel&id=p-hello"
            ),
            "got {events:?}",
        );
    }

    #[tokio::test]
    async fn webview_create_panel_with_unknown_slot_rejects() {
        let (_runtime, peer_conn, _log) = wired_pair_with_webview_capture().await;
        let err = peer_conn
            .request(
                webview_method::CREATE_PANEL,
                Value::Map(vec![
                    (Value::String("panelId".into()), Value::String("p-x".into())),
                    (
                        Value::String("entry".into()),
                        Value::String("./x.html".into()),
                    ),
                    (Value::String("slot".into()), Value::String("nope".into())),
                ]),
            )
            .await
            .expect_err("unknown slot must reject");
        let msg = format!("{err}");
        assert!(msg.contains("unknown slot"), "got `{msg}`");
    }

    #[tokio::test]
    async fn webview_dispose_panel_notify_clears_registry_and_emits_event() {
        let (runtime, peer_conn, log) = wired_pair_with_webview_capture().await;
        // Seed a panel.
        peer_conn
            .request(
                webview_method::CREATE_PANEL,
                Value::Map(vec![
                    (Value::String("panelId".into()), Value::String("p-1".into())),
                    (
                        Value::String("entry".into()),
                        Value::String("./p.html".into()),
                    ),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(runtime.webviews().len(), 1);

        peer_conn
            .notify(
                webview_method::DISPOSE_PANEL,
                Value::Map(vec![(
                    Value::String("panelId".into()),
                    Value::String("p-1".into()),
                )]),
            )
            .await
            .unwrap();
        wait_until(|| runtime.webviews().is_empty()).await;
        assert!(runtime.webviews().is_empty());
        // Two emitter events: PanelCreated + PanelDisposed.
        let events = log.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[1],
            crate::extensions::api::webview::WebviewEvent::PanelDisposed { panel_id } if panel_id == "p-1"
        ));
    }

    #[tokio::test]
    async fn webview_post_message_notify_emits_message_event() {
        let (runtime, peer_conn, log) = wired_pair_with_webview_capture().await;
        peer_conn
            .request(
                webview_method::CREATE_PANEL,
                Value::Map(vec![
                    (Value::String("panelId".into()), Value::String("p-2".into())),
                    (
                        Value::String("entry".into()),
                        Value::String("./p.html".into()),
                    ),
                ]),
            )
            .await
            .unwrap();
        // Drain the PanelCreated event from the capture so the next
        // assertion sees only the Message.
        log.lock().unwrap().clear();

        peer_conn
            .notify(
                webview_method::POST_MESSAGE,
                Value::Map(vec![
                    (Value::String("panelId".into()), Value::String("p-2".into())),
                    (
                        Value::String("payload".into()),
                        Value::Map(vec![(
                            Value::String("note".into()),
                            Value::String("hi".into()),
                        )]),
                    ),
                ]),
            )
            .await
            .unwrap();
        wait_until(|| !log.lock().unwrap().is_empty()).await;
        let events = log.lock().unwrap().clone();
        match &events[..] {
            [crate::extensions::api::webview::WebviewEvent::Message { panel_id, payload }] => {
                assert_eq!(panel_id, "p-2");
                assert_eq!(payload.get("note").and_then(|v| v.as_str()), Some("hi"),);
            }
            other => panic!("expected one Message event, got {other:?}"),
        }
        let _ = runtime; // keep alive
    }

    #[tokio::test]
    async fn webview_set_visible_notify_emits_visibility_event() {
        let (runtime, peer_conn, log) = wired_pair_with_webview_capture().await;
        peer_conn
            .request(
                webview_method::CREATE_PANEL,
                Value::Map(vec![
                    (Value::String("panelId".into()), Value::String("p-3".into())),
                    (
                        Value::String("entry".into()),
                        Value::String("./p.html".into()),
                    ),
                ]),
            )
            .await
            .unwrap();
        log.lock().unwrap().clear();

        peer_conn
            .notify(
                webview_method::SET_VISIBLE,
                Value::Map(vec![
                    (Value::String("panelId".into()), Value::String("p-3".into())),
                    (Value::String("visible".into()), Value::Boolean(true)),
                ]),
            )
            .await
            .unwrap();
        wait_until(|| !log.lock().unwrap().is_empty()).await;
        let events = log.lock().unwrap().clone();
        assert!(matches!(
            events.as_slice(),
            [crate::extensions::api::webview::WebviewEvent::VisibilityChanged {
                panel_id, visible,
            }] if panel_id == "p-3" && *visible
        ));
        // And the registry's PanelView reflects the new state.
        let pv = runtime.webviews().get("p-3").unwrap().unwrap();
        assert!(pv.visible);
    }

    #[tokio::test]
    async fn forward_panel_message_delivers_to_owning_extension() {
        // Capture the `webview/onDidReceiveMessage` notify on the peer
        // side and verify the runtime routes it to the right ext.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", manifest.clone());

        type Captured = Arc<StdMutex<Vec<(String, serde_json::Value)>>>;
        let captured: Captured = Arc::new(StdMutex::new(Vec::new()));
        let cap_c = captured.clone();
        let peer_server = RpcServer::builder()
            .on_notify(webview_method::ON_DID_RECEIVE_MESSAGE, move |params| {
                let cap = cap_c.clone();
                async move {
                    let panel_id = extract_str_field(&params, "panelId").unwrap_or_default();
                    let payload = lookup_field(&params, "payload")
                        .map(|v| rmpv_to_json(&v))
                        .unwrap_or(serde_json::Value::Null);
                    cap.lock().unwrap().push((panel_id, payload));
                    Ok(())
                }
            })
            .build();

        let runtime_rpc = runtime.build_per_extension_handlers("alice.x", &manifest);
        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, runtime_rpc);
        let (_peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);

        // Seed a panel and then drive forward_panel_message as the
        // renderer-side bridge would.
        runtime
            .webviews()
            .create(crate::extensions::api::webview::CreatePanelArgs {
                ext_id: "alice.x".into(),
                panel_id: "p-fwd".into(),
                title: "T".into(),
                slot: crate::extensions::api::webview::PanelSlot::Sidebar,
                entry: "./e.html".into(),
            })
            .unwrap();
        runtime
            .forward_panel_message("p-fwd", serde_json::json!({"from": "iframe"}))
            .await
            .expect("forward must succeed");

        for _ in 0..50 {
            if !captured.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let caps = captured.lock().unwrap().clone();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].0, "p-fwd");
        assert_eq!(
            caps[0].1.get("from").and_then(|v| v.as_str()),
            Some("iframe"),
        );
    }

    #[tokio::test]
    async fn forward_panel_message_for_unknown_panel_errors() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let err = runtime
            .forward_panel_message("ghost", serde_json::json!({}))
            .await
            .expect_err("missing panel must reject");
        assert!(
            matches!(err, ExtensionError::BadContribution { .. }),
            "got {err:?}",
        );
    }

    /// Register one sidebar view owned by `alice.x` directly in the
    /// registry (what `registerWebviewViewProvider` does over the wire).
    fn seed_view(runtime: &ExtensionRuntime, view_id: &str, owner: &str) {
        runtime
            .sidebars()
            .register(crate::extensions::api::sidebar::SidebarViewEntry {
                view_id: view_id.into(),
                owning_ext: owner.into(),
                title: "V".into(),
                icon: None,
                entry: "./v.html".into(),
            })
            .unwrap();
    }

    #[tokio::test]
    async fn forward_panel_message_routes_registered_view_to_provider() {
        // A view frame's iframe→ext post arrives as forward_panel_message
        // keyed by the viewId (not a panel). It must fall back to the
        // sidebar-view registry and route `webviewView/onDidReceiveMessage`.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        seed_view(&runtime, "alice.x.view", "alice.x");

        type Captured = Arc<StdMutex<Vec<(String, serde_json::Value)>>>;
        let captured: Captured = Arc::new(StdMutex::new(Vec::new()));
        let cap_c = captured.clone();
        let peer_server = RpcServer::builder()
            .on_notify(webview_view_method::ON_DID_RECEIVE_MESSAGE, move |params| {
                let cap = cap_c.clone();
                async move {
                    let view_id = extract_str_field(&params, "viewId").unwrap_or_default();
                    let payload = lookup_field(&params, "payload")
                        .map(|v| rmpv_to_json(&v))
                        .unwrap_or(serde_json::Value::Null);
                    cap.lock().unwrap().push((view_id, payload));
                    Ok(())
                }
            })
            .build();

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, RpcServer::builder().build());
        let (_peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);

        runtime
            .forward_panel_message("alice.x.view", serde_json::json!({"from": "view-iframe"}))
            .await
            .expect("view route must succeed");

        for _ in 0..50 {
            if !captured.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let caps = captured.lock().unwrap().clone();
        assert_eq!(caps.len(), 1, "expected one onDidReceiveMessage notify");
        assert_eq!(caps[0].0, "alice.x.view");
        assert_eq!(
            caps[0].1.get("from").and_then(|v| v.as_str()),
            Some("view-iframe"),
        );
    }

    #[tokio::test]
    async fn resolve_view_notifies_owner_and_noops_for_unknown() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        seed_view(&runtime, "alice.x.view", "alice.x");

        type Captured = Arc<StdMutex<Vec<String>>>;
        let captured: Captured = Arc::new(StdMutex::new(Vec::new()));
        let cap_c = captured.clone();
        let peer_server = RpcServer::builder()
            .on_notify(webview_view_method::RESOLVE, move |params| {
                let cap = cap_c.clone();
                async move {
                    let view_id = extract_str_field(&params, "viewId").unwrap_or_default();
                    cap.lock().unwrap().push(view_id);
                    Ok(())
                }
            })
            .build();

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, RpcServer::builder().build());
        let (_peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);

        // Unknown view → benign no-op (Ok), no notify.
        runtime
            .resolve_view("nope.view")
            .await
            .expect("unknown view resolves to Ok");

        runtime
            .resolve_view("alice.x.view")
            .await
            .expect("resolve must succeed");

        for _ in 0..50 {
            if !captured.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let caps = captured.lock().unwrap().clone();
        assert_eq!(caps, vec!["alice.x.view".to_string()]);
    }

    #[tokio::test]
    async fn resolve_view_for_owner_without_host_is_noop() {
        // Declarative-only / not-yet-host-backed views: notify_extension
        // returns NotActivated, which resolve_view swallows.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        seed_view(&runtime, "alice.x.view", "alice.x");
        runtime
            .resolve_view("alice.x.view")
            .await
            .expect("resolve with no live host must be a no-op");
    }

    #[tokio::test]
    async fn change_view_visibility_notifies_owner_with_flag_and_noops_for_unknown() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        seed_view(&runtime, "alice.x.view", "alice.x");

        type Captured = Arc<StdMutex<Vec<(String, bool)>>>;
        let captured: Captured = Arc::new(StdMutex::new(Vec::new()));
        let cap_c = captured.clone();
        let peer_server = RpcServer::builder()
            .on_notify(
                webview_view_method::ON_DID_CHANGE_VISIBILITY,
                move |params| {
                    let cap = cap_c.clone();
                    async move {
                        let view_id = extract_str_field(&params, "viewId").unwrap_or_default();
                        let visible = params
                            .as_map()
                            .and_then(|m| {
                                m.iter()
                                    .find(|(k, _)| k.as_str() == Some("visible"))
                                    .and_then(|(_, v)| v.as_bool())
                            })
                            .unwrap_or(true);
                        cap.lock().unwrap().push((view_id, visible));
                        Ok(())
                    }
                },
            )
            .build();

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, RpcServer::builder().build());
        let (_peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);

        // Unknown view → benign no-op (Ok), no notify.
        runtime
            .change_view_visibility("nope.view", false)
            .await
            .expect("unknown view is Ok");

        runtime
            .change_view_visibility("alice.x.view", false)
            .await
            .expect("hide must succeed");
        runtime
            .change_view_visibility("alice.x.view", true)
            .await
            .expect("show must succeed");

        for _ in 0..50 {
            if captured.lock().unwrap().len() >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let caps = captured.lock().unwrap().clone();
        assert_eq!(
            caps,
            vec![
                ("alice.x.view".to_string(), false),
                ("alice.x.view".to_string(), true),
            ],
            "visibility notify must carry the per-call flag in order",
        );
    }

    #[tokio::test]
    async fn change_view_visibility_for_owner_without_host_is_noop() {
        // Declarative-only / not-yet-host-backed views: notify_extension
        // returns NotActivated, which change_view_visibility swallows.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        seed_view(&runtime, "alice.x.view", "alice.x");
        runtime
            .change_view_visibility("alice.x.view", false)
            .await
            .expect("visibility change with no live host must be a no-op");
    }

    #[tokio::test]
    async fn dispose_view_notifies_owner_and_noops_for_unknown() {
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        seed_view(&runtime, "alice.x.view", "alice.x");

        type Captured = Arc<StdMutex<Vec<String>>>;
        let captured: Captured = Arc::new(StdMutex::new(Vec::new()));
        let cap_c = captured.clone();
        let peer_server = RpcServer::builder()
            .on_notify(webview_view_method::ON_DID_DISPOSE, move |params| {
                let cap = cap_c.clone();
                async move {
                    let view_id = extract_str_field(&params, "viewId").unwrap_or_default();
                    cap.lock().unwrap().push(view_id);
                    Ok(())
                }
            })
            .build();

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, RpcServer::builder().build());
        let (_peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle_conn_only("alice.x", runtime_conn);

        // Unknown view → benign no-op (Ok), no notify.
        runtime
            .dispose_view("nope.view")
            .await
            .expect("unknown view disposes to Ok");

        runtime
            .dispose_view("alice.x.view")
            .await
            .expect("dispose must succeed");

        for _ in 0..50 {
            if !captured.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let caps = captured.lock().unwrap().clone();
        assert_eq!(caps, vec!["alice.x.view".to_string()]);
    }

    #[tokio::test]
    async fn dispose_view_for_owner_without_host_is_noop() {
        // Declarative-only / not-yet-host-backed views: notify_extension
        // returns NotActivated, which dispose_view swallows.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        seed_view(&runtime, "alice.x.view", "alice.x");
        runtime
            .dispose_view("alice.x.view")
            .await
            .expect("dispose with no live host must be a no-op");
    }

    #[tokio::test]
    async fn webview_view_post_message_emits_for_owner_and_rejects_others() {
        use crate::extensions::api::webview::WebviewEvent;

        // Capture WebviewEvent::Message emitted by the runtime's
        // `webviewView/postMessage` handler (ext → view direction).
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let manifest = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", manifest.clone());
        // The view is owned by alice.x; the handler we build is alice.x's.
        seed_view(&runtime, "alice.x.view", "alice.x");

        let events: Arc<StdMutex<Vec<WebviewEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let cap = events.clone();
        runtime.set_webview_emitter(Arc::new(move |ev| cap.lock().unwrap().push(ev)));

        let runtime_rpc = runtime.build_per_extension_handlers("alice.x", &manifest);
        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (_runtime_conn, _t1) = Connection::open(a_r, a_w, runtime_rpc);
        let (peer_conn, _t2) = Connection::open(b_r, b_w, RpcServer::builder().build());

        // Owner posts → one Message emitted, keyed by viewId.
        peer_conn
            .notify(
                webview_view_method::POST_MESSAGE,
                Value::Map(vec![
                    (
                        Value::String("viewId".into()),
                        Value::String("alice.x.view".into()),
                    ),
                    (
                        Value::String("payload".into()),
                        Value::Map(vec![(
                            Value::String("hi".into()),
                            Value::String("view".into()),
                        )]),
                    ),
                ]),
            )
            .await
            .expect("notify sent");

        for _ in 0..50 {
            if !events.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let evs = events.lock().unwrap().clone();
        match &evs[..] {
            [WebviewEvent::Message { panel_id, payload }] => {
                assert_eq!(panel_id, "alice.x.view");
                assert_eq!(payload.get("hi").and_then(|v| v.as_str()), Some("view"));
            }
            other => panic!("expected one Message event, got {other:?}"),
        }

        // A post for a view this handler's ext does NOT own emits nothing.
        seed_view(&runtime, "bob.y.view", "bob.y");
        peer_conn
            .notify(
                webview_view_method::POST_MESSAGE,
                Value::Map(vec![(
                    Value::String("viewId".into()),
                    Value::String("bob.y.view".into()),
                )]),
            )
            .await
            .expect("notify sent");
        // Give the (rejected) notify time to be processed.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "cross-owner post must not emit a second Message",
        );
    }

    #[tokio::test]
    async fn cross_extension_post_message_is_rejected_at_ownership() {
        // alice.x creates a panel; bob.y tries to postMessage to it.
        // The Registry's owner check rejects without firing any event.
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let alice_m = alice_x_manifest_all_six();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("alice.x", alice_m.clone());
        let bob_m = Manifest::from_json(
            r#"{
                "id": "bob.y", "name": "Y", "version": "0.1.0",
                "publisher": "bob",
                "engines": { "cronymax": "^1.0" },
                "main": "./m.js",
                "activationEvents": [],
                "contributes": {}
            }"#,
        )
        .unwrap();
        runtime
            .state
            .registry
            .lock()
            .insert_for_test("bob.y", bob_m.clone());

        let alice_rpc = runtime.build_per_extension_handlers("alice.x", &alice_m);
        let bob_rpc = runtime.build_per_extension_handlers("bob.y", &bob_m);
        let (a, b) = duplex(8192);
        let (c, d) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (c_r, c_w) = split(c);
        let (d_r, d_w) = split(d);
        let (alice_conn, _t1) = Connection::open(a_r, a_w, alice_rpc);
        let (alice_peer, _t2) = Connection::open(b_r, b_w, RpcServer::builder().build());
        let (bob_conn, _t3) = Connection::open(c_r, c_w, bob_rpc);
        let (bob_peer, _t4) = Connection::open(d_r, d_w, RpcServer::builder().build());
        runtime.install_test_handle_conn_only("alice.x", alice_conn);
        runtime.install_test_handle_conn_only("bob.y", bob_conn);

        // alice creates a panel.
        alice_peer
            .request(
                webview_method::CREATE_PANEL,
                Value::Map(vec![
                    (
                        Value::String("panelId".into()),
                        Value::String("alice-panel".into()),
                    ),
                    (
                        Value::String("entry".into()),
                        Value::String("./a.html".into()),
                    ),
                ]),
            )
            .await
            .unwrap();

        // bob tries to push a message to alice's panel — notify swallows
        // the error platform-side, but the registry stays unmodified
        // and the renderer never sees a Message event from this attempt.
        bob_peer
            .notify(
                webview_method::POST_MESSAGE,
                Value::Map(vec![
                    (
                        Value::String("panelId".into()),
                        Value::String("alice-panel".into()),
                    ),
                    (
                        Value::String("payload".into()),
                        Value::String("evil".into()),
                    ),
                ]),
            )
            .await
            .unwrap();
        // Let the notify dispatch land then ensure no message-event was
        // emitted for alice-panel.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // The owner_of lookup still returns alice.x (panel unchanged).
        assert_eq!(
            runtime
                .webviews()
                .owner_of("alice-panel")
                .unwrap()
                .as_deref(),
            Some("alice.x"),
        );
    }

    #[tokio::test]
    async fn deactivate_disposes_webview_panels() {
        // create a panel, then unwind via the same path deactivate
        // takes (no real spawn needed — we exercise the registry
        // dispose_all_for + emitter directly).
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        runtime
            .webviews()
            .create(crate::extensions::api::webview::CreatePanelArgs {
                ext_id: "alice.x".into(),
                panel_id: "p1".into(),
                title: "T".into(),
                slot: crate::extensions::api::webview::PanelSlot::Sidebar,
                entry: "./e.html".into(),
            })
            .unwrap();
        runtime
            .webviews()
            .create(crate::extensions::api::webview::CreatePanelArgs {
                ext_id: "bob.y".into(),
                panel_id: "b1".into(),
                title: "T".into(),
                slot: crate::extensions::api::webview::PanelSlot::Sidebar,
                entry: "./e.html".into(),
            })
            .unwrap();
        assert_eq!(runtime.webviews().len(), 2);
        let n = runtime.webviews().dispose_all_for("alice.x").unwrap();
        assert_eq!(n, 1);
        assert_eq!(runtime.webviews().len(), 1);
        assert_eq!(
            runtime.webviews().owner_of("b1").unwrap().as_deref(),
            Some("bob.y"),
        );
    }
}
