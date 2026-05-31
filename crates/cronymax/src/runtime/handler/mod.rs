//! Dispatch handler backed by [`RuntimeAuthority`]. Replaces the
//! placeholder `EchoHandler` so the protocol surface is wired to real
//! runtime authority (tasks 4.2, 4.3 wired into the dispatch loop).
//!
//! Responsibilities:
//!
//!   * Translate `ControlRequest` variants into authority operations
//!     and map `AuthorityError` onto `ControlError`.
//!   * On `Subscribe`, open a runtime subscription and spawn a fan-out
//!     task that pumps events from the per-subscription receiver into
//!     the [`ResponseSink`] as `RuntimeToClient::Event` messages.
//!   * Track active fan-out tasks per subscription so `Unsubscribe`
//!     and disconnect both shut them down cleanly.
//!
//! Capability replies are accepted but not yet routed to a waiter —
//! capability *issuance* lands with task 6.x.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::protocol::capabilities::CapabilityResponse;
use crate::protocol::control::{ControlRequest, ControlResponse};
use crate::protocol::dispatch::{Handler, ResponseSink};
use crate::protocol::envelope::{CorrelationId, SubscriptionId};
use crate::sandbox::policy::SandboxPolicy;

use super::authority::{AuthorityError, RuntimeAuthority};
use super::state::RunId;
use crate::runtime::agent_runner::AgentRunner;
use crate::runtime::run_context::RunContext;
use crate::runtime::services::RuntimeServices;

mod contribution_ops;
mod document_ops;
mod extension_ops;
mod flow_ops;
mod helpers;
mod registry_ops;
mod review_ops;
mod run_ops;
mod run_start;
mod session_ops;
mod subscription;
mod terminal_ops;
mod workspace_ops;

/// Adapter that turns a [`RuntimeAuthority`] into a dispatch
/// [`Handler`].
pub struct RuntimeHandler {
    authority: RuntimeAuthority,
    /// Composition root — factories, flow registry, memory manager.
    services: Arc<RuntimeServices>,
    /// Agent runner that replaces `spawn_agent_loop` / `spawn_chat_turn`.
    agent_runner: AgentRunner,
    /// Workspace roots passed at construction time (from `StoragePaths`).
    workspace_roots: Vec<PathBuf>,
    /// Profile+workspace-scoped cache dir (`workspace_cache_dir` from `StoragePaths`).
    /// Used to construct the `ChatStore`.
    workspace_cache_dir: Option<PathBuf>,
    /// Sandbox policy derived from `RuntimeConfig.sandbox`; `None` = permissive.
    sandbox_policy: Option<Arc<SandboxPolicy>>,
    /// Kept once `on_connected` runs so subscribe-spawned fan-out tasks
    /// can reach back into the transport.
    sink: Mutex<Option<ResponseSink>>,
    /// Per-subscription fan-out task handles; aborted on unsubscribe
    /// or disconnect so we don't leak background tokio tasks.
    fanout: Mutex<HashMap<SubscriptionId, JoinHandle<()>>>,
    /// Per-flow-run contexts keyed by `flow_run_id` so `ResolveReview`
    /// can look up the `FlowRuntime` for a given flow run.
    flow_contexts: Mutex<HashMap<String /* flow_run_id */, RunContext>>,
    /// Maps flow_run_id → the original agent RunId from StartRun.
    /// Used to emit `flow.agent.notify` Raw events back to the chat subscription
    /// that the browser is already listening to, without needing an LLM turn.
    flow_run_to_agent_run: Mutex<HashMap<String /* flow_run_id */, RunId>>,
    /// One-shot senders awaiting a `CapabilityReply` from the C++ host.
    /// Keyed by the `CorrelationId` that was sent with the `CapabilityCall`.
    pending_capabilities:
        Mutex<HashMap<CorrelationId, tokio::sync::oneshot::Sender<CapabilityResponse>>>,
}

impl std::fmt::Debug for RuntimeHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeHandler").finish_non_exhaustive()
    }
}

impl RuntimeHandler {
    /// Primary constructor — wires up the handler from the composition root.
    /// Called by `lifecycle::Runtime::connect()`.
    pub fn from_services(
        services: Arc<RuntimeServices>,
        workspace_roots: Vec<PathBuf>,
        workspace_cache_dir: PathBuf,
        sandbox_policy: Option<SandboxPolicy>,
    ) -> Self {
        let authority = services.authority.clone();
        let agent_runner = AgentRunner::new(Arc::clone(&services));
        Self {
            authority,
            services,
            agent_runner,
            workspace_roots,
            workspace_cache_dir: Some(workspace_cache_dir),
            sandbox_policy: sandbox_policy.map(Arc::new),
            sink: Mutex::new(None),
            fanout: Mutex::new(HashMap::new()),
            flow_contexts: Mutex::new(HashMap::new()),
            flow_run_to_agent_run: Mutex::new(HashMap::new()),
            pending_capabilities: Mutex::new(HashMap::new()),
        }
    }

    /// Legacy constructor — kept for call sites that don't yet have
    /// `RuntimeServices`. Builds minimal services internally.
    #[deprecated(note = "use RuntimeHandler::from_services")]
    #[allow(deprecated)]
    pub fn new(authority: RuntimeAuthority, workspace_roots: Vec<PathBuf>) -> Self {
        Self::with_policy(authority, workspace_roots, None)
    }

    /// Construct with an explicit sandbox policy (built from `RuntimeConfig.sandbox`).
    #[deprecated(note = "use RuntimeHandler::from_services")]
    #[allow(deprecated)]
    pub fn with_policy(
        authority: RuntimeAuthority,
        workspace_roots: Vec<PathBuf>,
        sandbox_policy: Option<SandboxPolicy>,
    ) -> Self {
        Self::with_policy_and_managers(authority, workspace_roots, sandbox_policy, None)
    }

    /// Construct with an explicit sandbox policy and an optional shared terminal
    /// managers map.
    #[deprecated(note = "use RuntimeHandler::from_services")]
    pub fn with_policy_and_managers(
        authority: RuntimeAuthority,
        workspace_roots: Vec<PathBuf>,
        sandbox_policy: Option<SandboxPolicy>,
        terminal_managers: Option<
            Arc<Mutex<HashMap<String, crate::terminal::SharedPtySessionManager>>>,
        >,
    ) -> Self {
        let services = RuntimeServices::new_minimal(
            authority.clone(),
            terminal_managers.unwrap_or_else(|| Arc::new(Mutex::new(HashMap::new()))),
        );
        let agent_runner = AgentRunner::new(Arc::clone(&services));
        Self {
            authority,
            services,
            agent_runner,
            workspace_roots,
            workspace_cache_dir: None,
            sandbox_policy: sandbox_policy.map(Arc::new),
            sink: Mutex::new(None),
            fanout: Mutex::new(HashMap::new()),
            flow_contexts: Mutex::new(HashMap::new()),
            flow_run_to_agent_run: Mutex::new(HashMap::new()),
            pending_capabilities: Mutex::new(HashMap::new()),
        }
    }

    /// Configure the workspace cache directory (from `StoragePaths.workspace_cache_dir`).
    /// Call this before the handler is used for `start_run`.
    pub fn set_workspace_cache_dir(&mut self, dir: PathBuf) {
        self.workspace_cache_dir = Some(dir);
    }

    pub fn authority(&self) -> &RuntimeAuthority {
        &self.authority
    }

    /// Issue a `CapabilityCall` to the C++ host and wait for the matching
    /// `CapabilityReply`.  The caller can `.await` the returned future;
    /// `handle_capability_reply` will resolve it when the reply arrives.
    pub async fn call_capability(
        self: &Arc<Self>,
        request: crate::protocol::capabilities::CapabilityRequest,
    ) -> anyhow::Result<CapabilityResponse> {
        use crate::protocol::envelope::RuntimeToClient;

        let id = CorrelationId::new();
        let (tx, rx) = tokio::sync::oneshot::channel::<CapabilityResponse>();
        self.pending_capabilities.lock().insert(id, tx);

        let sink = self
            .sink
            .lock()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("call_capability: no active transport sink"))?;
        sink.send(RuntimeToClient::CapabilityCall { id, request })
            .await
            .map_err(|_| anyhow::anyhow!("call_capability: transport sink closed"))?;

        // Cap how long we wait for C++ to reply.  Without a timeout, a
        // capability call that C++ never answers (e.g. because the renderer
        // is busy or the IPC reply is dropped) parks the agent loop
        // indefinitely.  On timeout we clean up the pending entry and return
        // an error; the agent loop surfaces it as a run failure, which
        // unblocks the frontend immediately.
        const CAPABILITY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
        match tokio::time::timeout(CAPABILITY_TIMEOUT, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(anyhow::anyhow!(
                "call_capability: sender dropped (disconnected?)"
            )),
            Err(_elapsed) => {
                self.pending_capabilities.lock().remove(&id);
                tracing::warn!(
                    correlation_id = %id,
                    timeout_secs = CAPABILITY_TIMEOUT.as_secs(),
                    "call_capability: timed out — C++ never replied; failing the run"
                );
                Err(anyhow::anyhow!(
                    "call_capability: timed out after {}s waiting for C++ capability reply",
                    CAPABILITY_TIMEOUT.as_secs()
                ))
            }
        }
    }
}

#[async_trait]
impl Handler for RuntimeHandler {
    async fn on_connected(&self, sink: ResponseSink) {
        *self.sink.lock() = Some(sink);
    }

    async fn handle_control(&self, _id: CorrelationId, request: ControlRequest) -> ControlResponse {
        match request {
            ControlRequest::Ping => ControlResponse::Pong,
            ControlRequest::CancelRun { run_id } => self.run_op(&run_id, |a, id| a.cancel_run(id)),
            ControlRequest::PauseRun { run_id } => self.run_op(&run_id, |a, id| a.pause_run(id)),
            req @ ControlRequest::Subscribe { .. } => self.handle_subscribe(req),
            req @ ControlRequest::Unsubscribe { .. } => self.handle_unsubscribe(req),
            req @ ControlRequest::StartRun { .. } => self.handle_start_run(req).await,
            req @ ControlRequest::ResumeRun { .. } => self.handle_resume_run(req).await,
            req @ ControlRequest::SwapMemory { .. } => self.handle_swap_memory(req),
            req @ ControlRequest::PostInput { .. } => self.handle_post_input(req),
            req @ ControlRequest::ResolveReview { .. } => self.handle_resolve_review(req).await,
            req @ ControlRequest::WorkspaceLayout { .. } => self.handle_workspace_layout(req).await,
            req @ ControlRequest::FileRead { .. } => self.handle_file_read(req).await,
            req @ ControlRequest::FileWrite { .. } => self.handle_file_write(req).await,
            req @ ControlRequest::FlowList { .. } => self.handle_flow_list(req).await,
            req @ ControlRequest::FlowLoad { .. } => self.handle_flow_load(req).await,
            req @ ControlRequest::FlowSave { .. } => self.handle_flow_save(req).await,
            req @ ControlRequest::ContributionList { .. } => {
                self.handle_contribution_list(req).await
            }
            req @ ControlRequest::ContributionEnumerate { .. } => {
                self.handle_contribution_enumerate(req).await
            }
            req @ ControlRequest::ContributionLoad { .. } => {
                self.handle_contribution_load(req).await
            }
            req @ ControlRequest::ContributionSave { .. } => {
                self.handle_contribution_save(req).await
            }
            req @ ControlRequest::ContributionDelete { .. } => {
                self.handle_contribution_delete(req).await
            }
            req @ ControlRequest::DocTypeList { .. } => self.handle_doc_type_list(req).await,
            req @ ControlRequest::DocTypeLoad { .. } => self.handle_doc_type_load(req).await,
            req @ ControlRequest::DocTypeSave { .. } => self.handle_doc_type_save(req).await,
            req @ ControlRequest::DocTypeDelete { .. } => self.handle_doc_type_delete(req).await,
            req @ ControlRequest::TerminalStart { .. } => self.handle_terminal_start(req).await,
            req @ ControlRequest::TerminalInput { .. } => self.handle_terminal_input(req).await,
            req @ ControlRequest::TerminalResize { .. } => self.handle_terminal_resize(req).await,
            req @ ControlRequest::TerminalStop { .. } => self.handle_terminal_stop(req).await,
            req @ ControlRequest::DocumentList { .. } => self.handle_document_list(req).await,
            req @ ControlRequest::DocumentRead { .. } => self.handle_document_read(req).await,
            req @ ControlRequest::DocumentSubmit { .. } => self.handle_document_submit(req).await,
            req @ ControlRequest::DocumentSuggestionApply { .. } => {
                self.handle_document_suggestion_apply(req).await
            }
            req @ ControlRequest::MentionParse { .. } => self.handle_mention_parse(req).await,
            req @ ControlRequest::GetSpaceSnapshot { .. } => {
                self.handle_get_space_snapshot(req).await
            }
            req @ ControlRequest::SessionList { .. } => self.handle_session_list(req).await,
            req @ ControlRequest::SessionThreadInspect { .. } => {
                self.handle_session_thread_inspect(req).await
            }
            req @ ControlRequest::ListProviderModels { .. } => {
                self.handle_list_provider_models(req).await
            }
            req @ ControlRequest::FlowRunGetPendingReviews { .. } => {
                self.handle_flow_run_get_pending_reviews(req).await
            }
            req @ ControlRequest::GetSessionPendingActions { .. } => {
                self.handle_get_session_pending_actions(req).await
            }
            req @ ControlRequest::FlowRunApprove { .. } => self.handle_flow_run_approve(req).await,
            req @ ControlRequest::FlowRunRequestChanges { .. } => {
                self.handle_flow_run_request_changes(req).await
            }
            req @ ControlRequest::ExtensionWebviewPost { .. } => {
                self.handle_extension_webview_post(req).await
            }
            req @ ControlRequest::ExtensionRendererSetHeight { .. } => {
                self.handle_extension_renderer_set_height(req).await
            }
            req @ ControlRequest::ExtensionDeactivate { .. } => {
                self.handle_extension_deactivate(req).await
            }
            req @ ControlRequest::ExtensionViewResolve { .. } => {
                self.handle_extension_view_resolve(req).await
            }
            req @ ControlRequest::FlowSaveYaml { .. } => self.handle_flow_save_yaml(req).await,
            req @ ControlRequest::FlowSaveLayout { .. } => self.handle_flow_save_layout(req).await,
            req @ ControlRequest::BlackboardInject { .. } => {
                self.handle_blackboard_inject(req).await
            }
            req @ ControlRequest::SessionRename { .. } => self.handle_session_rename(req).await,
        }
    }

    async fn handle_capability_reply(&self, id: CorrelationId, response: CapabilityResponse) {
        // Route the reply to whoever is awaiting this correlation id.
        if let Some(tx) = self.pending_capabilities.lock().remove(&id) {
            if tx.send(response).is_err() {
                debug!(%id, "capability reply: waiter already dropped");
            }
        } else {
            warn!(%id, "capability reply: no pending waiter (already resolved or unexpected id)");
        }
    }

    async fn on_disconnected(&self) {
        // Drop the sink so any in-flight fan-out send fails fast and
        // the tasks tear themselves down.
        *self.sink.lock() = None;
        let mut tasks = self.fanout.lock();
        for (_, t) in tasks.drain() {
            t.abort();
        }
    }
}

impl RuntimeHandler {
    fn run_op<F>(&self, run_id_str: &str, op: F) -> ControlResponse
    where
        F: FnOnce(&RuntimeAuthority, RunId) -> Result<(), AuthorityError>,
    {
        let id = match helpers::parse_run(run_id_str) {
            Ok(r) => r,
            Err(resp) => return resp,
        };
        match op(&self.authority, id) {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: helpers::authority_err_to_control(e, None, Some(run_id_str)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::extensions::api::agents::ProviderEntry;
    use crate::extensions::{ExtensionRegistry, ExtensionRuntime};
    use crate::protocol::control::ControlError;
    use crate::protocol::dispatch::run as dispatch_run;
    use crate::protocol::envelope::{ClientToRuntime, RuntimeToClient};
    use crate::protocol::transport::memory;
    use crate::protocol::version::PROTOCOL_VERSION;
    use crate::runtime::state::{RunStatus, Space, SpaceId};

    async fn handshake(client: &memory::ClientEnd) {
        client
            .send(ClientToRuntime::Hello {
                protocol: PROTOCOL_VERSION,
                client_name: "t".into(),
                client_version: "0".into(),
            })
            .await
            .unwrap();
        match client.recv().await.unwrap() {
            RuntimeToClient::Welcome { .. } => {}
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_run_then_subscribe_streams_status_event() {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        let handler = Arc::new(RuntimeHandler::from_services(
            RuntimeServices::new_minimal(auth.clone(), Arc::new(Mutex::new(HashMap::new()))),
            vec![],
            std::env::temp_dir(),
            None,
        ));
        let (server, client) = memory::pair();
        let task = tokio::spawn({
            let handler = handler.clone();
            async move { dispatch_run(server, ArcAdapter(handler)).await }
        });

        handshake(&client).await;

        // Subscribe to *.
        let sub_id = CorrelationId::new();
        client
            .send(ClientToRuntime::Control {
                id: sub_id,
                request: ControlRequest::Subscribe { topic: "*".into() },
            })
            .await
            .unwrap();
        match client.recv().await.unwrap() {
            RuntimeToClient::Control {
                response: ControlResponse::Subscribed { .. },
                ..
            } => {}
            other => panic!("expected Subscribed, got {other:?}"),
        }

        // Start a run via control.
        let start_id = CorrelationId::new();
        client
            .send(ClientToRuntime::Control {
                id: start_id,
                request: ControlRequest::StartRun {
                    space_id: space_id.to_string(),
                    payload: serde_json::json!({}),
                    session_id: None,
                    session_name: None,
                    agent_id: None,
                    contribution_kind: None,
                    child_session_id: None,
                    goal: None,
                },
            })
            .await
            .unwrap();
        // An Event(RunStatus{pending}) may fire before RunStarted if the
        // authority emits an event synchronously during start_run. Capture
        // the genuinely-first event regardless of arrival order — it is the
        // one that carries sequence 0.
        let mut first_event = None;
        let _run_id = loop {
            match client.recv().await.unwrap() {
                RuntimeToClient::Control {
                    response: ControlResponse::RunStarted { run_id, .. },
                    ..
                } => break run_id,
                RuntimeToClient::Event { event, .. } => {
                    if first_event.is_none() {
                        first_event = Some(event);
                    }
                }
                other => panic!("expected RunStarted or Event, got {other:?}"),
            }
        };

        // Expect the resulting Event message on the subscription.
        let event = match first_event {
            Some(ev) => ev,
            None => match client.recv().await.unwrap() {
                RuntimeToClient::Event { event, .. } => event,
                other => panic!("expected Event, got {other:?}"),
            },
        };
        assert_eq!(event.sequence, 0);

        client.close().await;
        task.await.unwrap().unwrap();
    }

    // Local Arc-adapter copy so we don't need to expose
    // protocol::session::ArcHandler. Mirrors the production adapter.
    #[derive(Debug)]
    struct ArcAdapter(Arc<RuntimeHandler>);

    #[async_trait]
    impl Handler for ArcAdapter {
        async fn on_connected(&self, sink: ResponseSink) {
            self.0.on_connected(sink).await
        }
        async fn handle_control(
            &self,
            id: CorrelationId,
            request: ControlRequest,
        ) -> ControlResponse {
            self.0.handle_control(id, request).await
        }
        async fn handle_capability_reply(&self, id: CorrelationId, response: CapabilityResponse) {
            self.0.handle_capability_reply(id, response).await
        }
        async fn on_disconnected(&self) {
            self.0.on_disconnected().await
        }
    }

    // ── Activity panel ─────────────────────────────────────────────────

    /// 10.4 – `GetSpaceSnapshot` dispatch returns `SpaceSnapshot` with only
    /// the runs and reviews belonging to the requested space.
    #[tokio::test]
    async fn get_space_snapshot_dispatch() {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "test".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        // Start a run in the space.
        let run_id = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();

        let handler = RuntimeHandler::from_services(
            RuntimeServices::new_minimal(auth.clone(), Arc::new(Mutex::new(HashMap::new()))),
            vec![],
            std::env::temp_dir(),
            None,
        );
        let cid = CorrelationId::new();
        let resp = handler
            .handle_control(
                cid,
                ControlRequest::GetSpaceSnapshot {
                    space_id: space_id.to_string(),
                },
            )
            .await;

        match resp {
            ControlResponse::SpaceSnapshot {
                runs,
                pending_reviews,
            } => {
                assert_eq!(runs.len(), 1);
                let id_in_json = runs[0]
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                assert_eq!(id_in_json, run_id.to_string());
                assert_eq!(pending_reviews.len(), 0);
            }
            other => panic!("expected SpaceSnapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn contribution_list_hides_inactive_extension_and_includes_active_one() {
        use crate::extensions::contributions::{
            kind as kind_id, ContributionDescriptor, ContributionOwner,
        };

        let auth = RuntimeAuthority::in_memory();
        let extensions = ExtensionRuntime::new(ExtensionRegistry::default());
        // Two providers from two different extensions: alice is activated,
        // bob is dormant. The picker must see alice but not bob.
        for (ext_id, prov_id, label) in [
            ("alice.ext", "alice.agent", "Alice Agent"),
            ("bob.ext", "bob.agent", "Bob Agent"),
        ] {
            extensions.add_contribution(
                ContributionDescriptor::new(
                    kind_id::AGENTS_PROVIDER,
                    prov_id,
                    ContributionOwner::Extension {
                        ext_id: ext_id.into(),
                    },
                    label,
                )
                .with_metadata(serde_json::json!({
                    "id": prov_id,
                    "label": label,
                    "supports_models": true,
                    "supports_modes": false,
                    "supports_mcp": true,
                })),
            );
        }
        extensions.test_mark_activated("alice.ext");

        let services = RuntimeServices {
            authority: auth,
            flow_registry: Arc::new(crate::flow::FlowRuntimeRegistry::default()),
            llm_factory: Arc::new(crate::llm::factory::DefaultLlmProviderFactory::new()),
            capability_factory: Arc::new(crate::capability::factory::DefaultCapabilityFactory),
            terminal_managers: Arc::new(Mutex::new(HashMap::new())),
            memory_manager: None,
            extensions: Some(extensions),
        };
        let handler =
            RuntimeHandler::from_services(Arc::new(services), vec![], std::env::temp_dir(), None);

        let resp = handler
            .handle_control(
                CorrelationId::new(),
                ControlRequest::ContributionList {
                    workspace_root: std::env::temp_dir().display().to_string(),
                },
            )
            .await;

        let payload = match resp {
            ControlResponse::Data { payload } => payload,
            other => panic!("expected Data, got {other:?}"),
        };
        let contributions = payload["contributions"]
            .as_array()
            .expect("contributions array");
        let alice = contributions
            .iter()
            .find(|d| d["id"] == "alice.agent")
            .expect("active extension provider must appear");
        assert_eq!(alice["owner"]["type"], "extension");
        assert_eq!(alice["owner"]["extId"], "alice.ext");
        assert!(
            !contributions.iter().any(|d| d["id"] == "bob.agent"),
            "inactive extension provider must be hidden from the picker",
        );
    }

    #[tokio::test]
    async fn start_run_with_inactive_extension_provider_returns_invalid_state() {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        let extensions = ExtensionRuntime::new(ExtensionRegistry::default());
        extensions
            .providers()
            .register(ProviderEntry {
                provider_id: "alice.agent".into(),
                owning_ext: "alice.ext".into(),
                label: "Alice Agent".into(),
                icon: None,
                description: None,
                supports_models: false,
                supports_modes: false,
                supports_mcp: false,
            })
            .unwrap();

        let services = RuntimeServices {
            authority: auth,
            flow_registry: Arc::new(crate::flow::FlowRuntimeRegistry::default()),
            llm_factory: Arc::new(crate::llm::factory::DefaultLlmProviderFactory::new()),
            capability_factory: Arc::new(crate::capability::factory::DefaultCapabilityFactory),
            terminal_managers: Arc::new(Mutex::new(HashMap::new())),
            memory_manager: None,
            extensions: Some(extensions),
        };
        let handler =
            RuntimeHandler::from_services(Arc::new(services), vec![], std::env::temp_dir(), None);

        let resp = handler
            .handle_control(
                CorrelationId::new(),
                ControlRequest::StartRun {
                    space_id: space_id.to_string(),
                    payload: serde_json::json!({"task": "hello"}),
                    session_id: None,
                    session_name: None,
                    agent_id: Some("alice.agent".into()),
                    contribution_kind: Some(
                        crate::extensions::contributions::kind::AGENTS_PROVIDER.into(),
                    ),
                    child_session_id: None,
                    goal: None,
                },
            )
            .await;

        match resp {
            ControlResponse::Err {
                error: ControlError::InvalidState { message },
            } => {
                assert!(message.contains("alice.agent"), "message: {message}");
                assert!(message.contains("alice.ext"), "message: {message}");
                assert!(
                    message.contains("not activated"),
                    "expected activation-related message, got: {message}",
                );
            }
            other => panic!("expected InvalidState error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_run_for_flow_skips_extension_provider_dispatch() {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        let extensions = ExtensionRuntime::new(ExtensionRegistry::default());
        extensions
            .providers()
            .register(ProviderEntry {
                provider_id: "alice.agent".into(),
                owning_ext: "alice.ext".into(),
                label: "Alice Agent".into(),
                icon: None,
                description: None,
                supports_models: false,
                supports_modes: false,
                supports_mcp: false,
            })
            .unwrap();

        let services = RuntimeServices {
            authority: auth,
            flow_registry: Arc::new(crate::flow::FlowRuntimeRegistry::default()),
            llm_factory: Arc::new(crate::llm::factory::DefaultLlmProviderFactory::new()),
            capability_factory: Arc::new(crate::capability::factory::DefaultCapabilityFactory),
            terminal_managers: Arc::new(Mutex::new(HashMap::new())),
            memory_manager: None,
            extensions: Some(extensions),
        };
        let handler =
            RuntimeHandler::from_services(Arc::new(services), vec![], std::env::temp_dir(), None);

        // flow_id is present, so the extension-provider guard must not fire.
        // The run should be created (or fail for a flow-specific reason),
        // but never with the extension-provider InvalidState error.
        let resp = handler
            .handle_control(
                CorrelationId::new(),
                ControlRequest::StartRun {
                    space_id: space_id.to_string(),
                    payload: serde_json::json!({
                        "task": "hello",
                        "flow_id": "some-flow",
                    }),
                    session_id: None,
                    session_name: None,
                    agent_id: Some("alice.agent".into()),
                    contribution_kind: None,
                    child_session_id: None,
                    goal: None,
                },
            )
            .await;

        if let ControlResponse::Err {
            error: ControlError::InvalidState { ref message },
        } = resp
        {
            assert!(
                !message.contains("extension provider"),
                "flow path should not hit the extension provider guard, got: {message}",
            );
        }
    }

    /// `handle_start_run` must mirror the resolved agent id into the run
    /// spec so ResumeRun can recover it after a restart. Without this the
    /// resume path always falls back to the Crony builtin.
    #[tokio::test]
    async fn start_run_persists_agent_id_into_run_spec() {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        let services = RuntimeServices {
            authority: auth.clone(),
            flow_registry: Arc::new(crate::flow::FlowRuntimeRegistry::default()),
            llm_factory: Arc::new(crate::llm::factory::DefaultLlmProviderFactory::new()),
            capability_factory: Arc::new(crate::capability::factory::DefaultCapabilityFactory),
            terminal_managers: Arc::new(Mutex::new(HashMap::new())),
            memory_manager: None,
            extensions: None,
        };
        let handler =
            RuntimeHandler::from_services(Arc::new(services), vec![], std::env::temp_dir(), None);

        let resp = handler
            .handle_control(
                CorrelationId::new(),
                ControlRequest::StartRun {
                    space_id: space_id.to_string(),
                    payload: serde_json::json!({ "task": "hi" }),
                    session_id: None,
                    session_name: None,
                    agent_id: Some("my-agent".into()),
                    contribution_kind: None,
                    child_session_id: None,
                    goal: None,
                },
            )
            .await;
        match resp {
            ControlResponse::RunStarted { .. } => {}
            other => panic!("expected RunStarted, got {other:?}"),
        }

        let snap = auth.snapshot();
        let run = snap
            .runs
            .values()
            .next()
            .expect("the run should have been created");
        assert_eq!(
            run.spec.get("agent_id").and_then(|v| v.as_str()),
            Some("my-agent"),
            "resolved agent id must be mirrored into the run spec",
        );
    }

    /// Resuming a run that was driven by an extension provider must be
    /// rejected — symmetric to the StartRun guard. The extension's
    /// AgentSession does not survive the run leaving `Running`, so the
    /// native agent_loader reconstruction below would silently mis-run it.
    #[tokio::test]
    async fn resume_run_for_extension_provider_returns_invalid_state() {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        // Manufacture a Paused run whose spec names an extension provider —
        // exactly the shape handle_start_run persists, then what
        // RuntimeAuthority::rehydrate leaves behind after a restart.
        let run_id = auth
            .start_run_with_session(
                space_id,
                None,
                serde_json::json!({
                    "agent_id": "alice.agent",
                    "contribution_kind": crate::extensions::contributions::kind::AGENTS_PROVIDER,
                    "task": "hi",
                }),
                None,
            )
            .unwrap();
        auth.pause_run(run_id).unwrap();

        let extensions = ExtensionRuntime::new(ExtensionRegistry::default());
        extensions
            .providers()
            .register(ProviderEntry {
                provider_id: "alice.agent".into(),
                owning_ext: "alice.ext".into(),
                label: "Alice Agent".into(),
                icon: None,
                description: None,
                supports_models: false,
                supports_modes: false,
                supports_mcp: false,
            })
            .unwrap();

        let services = RuntimeServices {
            authority: auth.clone(),
            flow_registry: Arc::new(crate::flow::FlowRuntimeRegistry::default()),
            llm_factory: Arc::new(crate::llm::factory::DefaultLlmProviderFactory::new()),
            capability_factory: Arc::new(crate::capability::factory::DefaultCapabilityFactory),
            terminal_managers: Arc::new(Mutex::new(HashMap::new())),
            memory_manager: None,
            extensions: Some(extensions),
        };
        let handler =
            RuntimeHandler::from_services(Arc::new(services), vec![], std::env::temp_dir(), None);

        let resp = handler
            .handle_control(
                CorrelationId::new(),
                ControlRequest::ResumeRun {
                    run_id: run_id.to_string(),
                },
            )
            .await;

        match resp {
            ControlResponse::Err {
                error: ControlError::InvalidState { message },
            } => {
                assert!(message.contains("alice.agent"), "message: {message}");
                assert!(message.contains("alice.ext"), "message: {message}");
                assert!(
                    message.contains("cannot be resumed"),
                    "expected resume-rejection message, got: {message}",
                );
            }
            other => panic!("expected InvalidState error, got {other:?}"),
        }

        // The rejected resume must leave the run Paused — never flipped to
        // Running and orphaned.
        assert!(
            matches!(
                auth.snapshot().runs.get(&run_id).map(|r| &r.status),
                Some(RunStatus::Paused)
            ),
            "rejected resume must leave the run Paused",
        );
    }
}
