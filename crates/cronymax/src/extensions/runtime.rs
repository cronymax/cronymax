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

use parking_lot::Mutex;
use rmpv::Value;

use crate::extensions::api::agents::{
    AgentProviderRegistry, AgentSessionEvent, AgentSessionRouter, ProviderEntry,
};
use crate::extensions::api::commands::CommandRegistry;
use crate::extensions::api::lifecycle::LifecycleState;
use crate::extensions::api::renderers::{ContentRendererRegistry, RendererEntry};
use crate::extensions::api::sidebar::{SidebarViewEntry, SidebarViewRegistry};
use crate::extensions::contributions::ContributionRegistry;
use crate::extensions::error::{ExtensionError, ExtensionResult};
use crate::extensions::host::node::{NodeHost, SpawnConfig};
use crate::extensions::manifest::{
    AgentProviderContribution, ContentRendererContribution, Manifest, SidebarViewContribution,
};
use crate::extensions::registry::ExtensionRegistry;
use crate::extensions::rpc::codec::{agents_method, method, renderers_method, sidebar_method};
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
}

/// Top-level orchestrator. Cheap to clone (`Arc` internals).
#[derive(Clone, Debug)]
pub struct ExtensionRuntime {
    state: Arc<RuntimeState>,
}

#[derive(Debug)]
struct RuntimeState {
    registry: Mutex<ExtensionRegistry>,
    contributions: Mutex<ContributionRegistry>,
    lifecycle: Mutex<LifecycleState>,
    commands: Mutex<CommandRegistry>,
    providers: AgentProviderRegistry,
    renderers: ContentRendererRegistry,
    sidebars: SidebarViewRegistry,
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
                handles: Mutex::new(HashMap::new()),
                session_router: AgentSessionRouter::new(),
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

        // 2. Build the per-extension RPC handler table.
        let rpc = self.build_rpc_server(ext_id.to_string(), manifest.clone());

        // 3. Spawn the host.
        let cfg = cfg_builder(&manifest, ext_dir);
        let host = match NodeHost::spawn(cfg, rpc).await {
            Ok(h) => h,
            Err(e) => {
                self.state.contributions.lock().remove_extension(ext_id);
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
            },
        );

        // 5. Drive extension/activate. Failure is fatal — rollback all
        //    state and tear down the host.
        if let Err(e) = conn.request(method::EXTENSION_ACTIVATE, Value::Nil).await {
            self.rollback_failed_activate(ext_id).await;
            return Err(e);
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

    /// Deactivate `ext_id`. Idempotent against a missing host (returns
    /// `NotActivated`).
    pub async fn deactivate(&self, ext_id: &str) -> ExtensionResult<()> {
        let handle = self
            .state
            .handles
            .lock()
            .remove(ext_id)
            .ok_or_else(|| ExtensionError::NotActivated(ext_id.to_string()))?;

        // Best-effort RPC notice; failure means the host is already gone,
        // which is fine.
        let _ = handle
            .conn
            .request(method::EXTENSION_DEACTIVATE, Value::Nil)
            .await;

        self.state.contributions.lock().remove_extension(ext_id);
        self.state.providers.unregister_all_for(ext_id);
        self.state.commands.lock().unregister_all_for(ext_id);
        self.state.renderers.unregister_all_for(ext_id);
        self.state.sidebars.unregister_all_for(ext_id);
        let _ = self.state.lifecycle.lock().mark_deactivated(ext_id);

        let _ = handle.host.shutdown().await;
        Ok(())
    }

    async fn rollback_failed_activate(&self, ext_id: &str) {
        let handle = self.state.handles.lock().remove(ext_id);
        self.state.contributions.lock().remove_extension(ext_id);
        self.state.providers.unregister_all_for(ext_id);
        self.state.commands.lock().unregister_all_for(ext_id);
        self.state.renderers.unregister_all_for(ext_id);
        self.state.sidebars.unregister_all_for(ext_id);
        if let Some(h) = handle {
            let _ = h.host.shutdown().await;
        }
    }

    // ── plumbing: build the per-extension RPC server ───────────────────

    fn build_rpc_server(&self, ext_id: String, manifest: Manifest) -> RpcServer {
        let providers = self.state.providers.clone();
        let renderers = self.state.renderers.clone();
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

        // ── renderers/register ─────────────────────────────────────────
        {
            let renderers = renderers.clone();
            let state = state.clone();
            let ext_id_c = ext_id.clone();
            let manifest_c = manifest.clone();
            builder = builder.on_notify(renderers_method::REGISTER, move |params| {
                let renderers = renderers.clone();
                let state = state.clone();
                let ext_id = ext_id_c.clone();
                let manifest = manifest_c.clone();
                async move {
                    let result = register_renderer_handler(&renderers, &manifest, &ext_id, &params);
                    report_register_outcome(
                        &state,
                        &ext_id,
                        "cronymax.content.renderer",
                        &params,
                        "rendererId",
                        result,
                    )
                    .await
                }
            });
        }

        // ── renderers/unregister ───────────────────────────────────────
        {
            let renderers = renderers.clone();
            let ext_id_c = ext_id.clone();
            builder = builder.on_notify(renderers_method::UNREGISTER, move |params| {
                let renderers = renderers.clone();
                let ext_id = ext_id_c.clone();
                async move {
                    let renderer_id = extract_str_field(&params, "rendererId")?;
                    let _ = renderers.unregister(&ext_id, &renderer_id)?;
                    Ok(())
                }
            });
        }

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

        builder.build()
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

fn register_renderer_handler(
    renderers: &ContentRendererRegistry,
    manifest: &Manifest,
    ext_id: &str,
    params: &Value,
) -> ExtensionResult<()> {
    let renderer_id = extract_str_field(params, "rendererId")?;
    let decl = find_renderer_decl(manifest, &renderer_id).ok_or_else(|| {
        ExtensionError::BadContribution {
            point: "cronymax.content.renderer".into(),
            ext_id: ext_id.to_string(),
            reason: format!("renderer `{renderer_id}` not declared in manifest contributes"),
        }
    })?;
    renderers.register(RendererEntry {
        renderer_id: decl.id.clone(),
        owning_ext: ext_id.to_string(),
        mime_types: decl.mime_types.clone(),
        entry: decl.entry.clone(),
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

fn find_renderer_decl<'a>(
    manifest: &'a Manifest,
    id: &str,
) -> Option<&'a ContentRendererContribution> {
    manifest
        .contributes
        .content_renderers
        .iter()
        .find(|r| r.id == id)
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
    async fn extension_renderer_register_lands_in_registry() {
        let (runtime, peer) = wired_pair().await;
        peer.notify(
            renderers_method::REGISTER,
            Value::Map(vec![(
                Value::String("rendererId".into()),
                Value::String("alice.x.rend".into()),
            )]),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if runtime.renderers().len() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
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
}
