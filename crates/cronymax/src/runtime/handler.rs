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
use tracing::{debug, info, warn};

use crate::agent_loop::tools::ToolDispatcher;
use crate::agent_loop::{
    maybe_compact, LoopConfig, ReactLoop, DEFAULT_RECENCY_TURNS, DEFAULT_THRESHOLD_PCT,
};
use crate::capability::agent_loader;
use crate::capability::dispatcher::HostCapabilityDispatcher;
use crate::capability::filesystem::{LocalFilesystem, WorkspaceScope};
use crate::capability::flow_tools::{register_flow_tools, SpawnAgentFn};
use crate::capability::notify::NullNotify;
use crate::capability::shell::LocalShell;
use crate::capability::submit_document::DocumentSubmitted;
use crate::llm::{
    copilot_auth, AnthropicConfig, AnthropicProvider, CapabilityResolver, OpenAiConfig,
    OpenAiProvider, ThinkingConfig,
};
use crate::protocol::capabilities::CapabilityResponse;
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse, ReviewDecision};
use crate::protocol::dispatch::{Handler, ResponseSink};
use crate::protocol::envelope::{CorrelationId, RuntimeToClient, SubscriptionId};
use crate::runtime::middleware::{
    LlmDurationStore, MiddlewareChain, TimingMiddleware, TokenAccumulatorMiddleware,
    ToolDurationStore, TraceEmitterMiddleware,
};
use crate::sandbox::policy::SandboxPolicy;
use uuid::Uuid;

use super::authority::{AuthorityError, RuntimeAuthority, SubscribeOutcome};
use super::state::{PermissionState, ReviewId, RunId, SessionId, Space, SpaceId};
use crate::capability::SandboxTier;
use crate::llm::LlmConfig;
use crate::runtime::agent_runner::AgentRunner;
use crate::runtime::run_context::RunContext;
use crate::runtime::services::RuntimeServices;

/// Build the workspace context block appended to an agent's system prompt
/// when `inject_workspace = true`. Minimal: just workspace path + tool names.
fn build_workspace_injection_block(
    workspace_path: &std::path::Path,
    tool_names: &[&str],
) -> String {
    let tools_line = if tool_names.is_empty() {
        "(none)".to_owned()
    } else {
        tool_names.join(", ")
    };
    format!(
        "\n---\nWorkspace: `{}`\nTools available: {}\nUse these tools to verify facts about the codebase. Never guess at structure.",
        workspace_path.display(),
        tools_line,
    )
}

/// Apply a per-run Anthropic effort override onto a model-derived
/// `ThinkingConfig`. Only the `Adaptive` variant carries an effort field;
/// other variants pass through unchanged. `None` for `override_effort`
/// leaves the existing value (typically the server default) intact.
fn apply_anthropic_effort_override(
    cfg: Option<ThinkingConfig>,
    override_effort: Option<&str>,
) -> Option<ThinkingConfig> {
    let Some(eff) = override_effort.map(str::trim).filter(|s| !s.is_empty()) else {
        return cfg;
    };
    match cfg {
        Some(ThinkingConfig::Adaptive { summarized, .. }) => Some(ThinkingConfig::Adaptive {
            summarized,
            effort: Some(eff.to_owned()),
        }),
        // Other variants don't take an effort; leave unchanged.
        other => other,
    }
}

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

/// Construct the default middleware chain for a `ReactLoop`.
///
/// Order: `[TimingMiddleware, TokenAccumulatorMiddleware, TraceEmitterMiddleware]`
///
/// `TimingMiddleware` runs first to record start times; `TokenAccumulatorMiddleware`
/// second to update `TurnContext.total_usage`; `TraceEmitterMiddleware` last
/// so it can read both timing data and updated usage.
fn build_middleware_chain(authority: RuntimeAuthority) -> Arc<MiddlewareChain> {
    let llm_durations: LlmDurationStore =
        Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
    let tool_durations: ToolDurationStore =
        Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
    let timing = Arc::new(TimingMiddleware::new(
        llm_durations.clone(),
        tool_durations.clone(),
    ));
    let token_accum = Arc::new(TokenAccumulatorMiddleware::new());
    let trace = Arc::new(TraceEmitterMiddleware::new(
        Arc::new(authority),
        llm_durations,
        tool_durations,
    ));
    Arc::new(MiddlewareChain(vec![timing, token_accum, trace]))
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

        rx.await
            .map_err(|_| anyhow::anyhow!("call_capability: sender dropped (disconnected?)"))
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

            ControlRequest::Subscribe { topic } => {
                let sink = match self.sink.lock().clone() {
                    Some(s) => s,
                    None => {
                        return ControlResponse::Err {
                            error: ControlError::Internal {
                                message: "subscribe before on_connected".into(),
                            },
                        }
                    }
                };
                let SubscribeOutcome { id, mut receiver } = self.authority.subscribe(topic);
                let task = tokio::spawn(async move {
                    while let Some(event) = receiver.recv().await {
                        if let Err(e) = sink
                            .send(RuntimeToClient::Event {
                                subscription: id,
                                event,
                            })
                            .await
                        {
                            warn!(%id, error = %e, "fan-out send failed; closing");
                            break;
                        }
                    }
                    debug!(%id, "fan-out task exiting");
                });
                self.fanout.lock().insert(id, task);
                ControlResponse::Subscribed { subscription: id }
            }

            ControlRequest::Unsubscribe { subscription } => {
                let removed = self.authority.unsubscribe(subscription);
                if let Some(task) = self.fanout.lock().remove(&subscription) {
                    task.abort();
                }
                if removed {
                    ControlResponse::Unsubscribed
                } else {
                    ControlResponse::Err {
                        error: ControlError::UnknownSubscription,
                    }
                }
            }

            ControlRequest::StartRun {
                space_id,
                payload,
                session_id,
                session_name,
                agent_id,
            } => {
                let space = match parse_space(&space_id) {
                    Ok(s) => s,
                    Err(resp) => return resp,
                };

                // Extract LLM config from payload (provided by the C++ host).
                let llm_obj = payload.get("llm");
                let base_url = llm_obj
                    .and_then(|l| l.get("base_url"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("https://api.openai.com/v1")
                    .to_string();
                let api_key = llm_obj
                    .and_then(|l| l.get("api_key"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let model = llm_obj
                    .and_then(|l| l.get("model"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("gpt-4o-mini")
                    .to_string();
                let provider_kind = llm_obj
                    .and_then(|l| l.get("provider_kind"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("openai_compat")
                    .to_string();
                // Optional OpenAI reasoning_effort: minimal/low/medium/high.
                // Source: payload.llm.reasoning_effort, set by the host from
                // either the active provider record or a per-message UI override.
                let reasoning_effort: Option<String> = llm_obj
                    .and_then(|l| l.get("reasoning_effort"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| {
                        matches!(s.as_str(), "minimal" | "low" | "medium" | "high" | "xhigh")
                    });
                // Optional Anthropic adaptive effort: low/medium/high/max.
                // Source: payload.llm.anthropic_effort, set by the host from a
                // per-message UI override. Only meaningful for native
                // Anthropic providers; ignored elsewhere.
                let anthropic_effort: Option<String> = llm_obj
                    .and_then(|l| l.get("anthropic_effort"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| matches!(s.as_str(), "low" | "medium" | "high" | "max"));
                let user_input = payload
                    .get("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let system_prompt = payload
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                // Extract workspace root from payload; fall back to first
                // configured root, then to a temp path.
                let workspace_root: PathBuf = payload
                    .get("workspace_root")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| self.workspace_roots.first().cloned())
                    .unwrap_or_else(std::env::temp_dir);

                // Optionally wire a FlowRuntime when the payload carries a
                // `flow_id` field (i.e. this is a flow-run invocation).
                let flow_id_opt = payload
                    .get("flow_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let initial_input = payload
                    .get("initial_input")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&user_input)
                    .to_string();

                info!(%base_url, %model, has_key = api_key.is_some(), "start_run: LLM config");

                // Pre-load agent definition for direct-chat runs (no flow_id).
                // Uses load_agent_with_builtin so the Crony builtin is always
                // available without a YAML file on disk.
                // Done before `start_run_with_session` so no yield-points exist
                // between run creation (RunStatus:pending) and RunStarted reply.
                let resolved_agent_id = agent_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(crate::crony::CronyBuiltin::ID);
                let preloaded_chat_agent_def: Option<crate::capability::agent_loader::AgentDef> =
                    if flow_id_opt.is_none() {
                        Some(
                            agent_loader::load_agent_with_builtin(
                                &workspace_root,
                                resolved_agent_id,
                            )
                            .await,
                        )
                    } else {
                        None
                    };

                // Resolve session: if session_id present, upsert the session
                // and retrieve the prior conversation thread from the ChatStore
                // (or fall back to the snapshot if ChatStore is not configured).
                let (maybe_session_id, prior_thread) = if let Some(ref sid) = session_id {
                    let s_id = SessionId::from(sid.as_str());
                    // Always upsert so run_ids tracking still works.
                    let _ = self.authority.get_or_create_session(
                        s_id.clone(),
                        space,
                        session_name.clone(),
                    );
                    // Load history from ChatStore if available, else fall back
                    // to the snapshot thread (legacy / no workspace_cache_dir).
                    // Flow runs always start fresh — never continue from the chat
                    // session thread. Each flow invocation is an independent task
                    // execution; reusing the prior thread causes role-alternation
                    // corruption when LLM calls fail and the broken history is
                    // flushed back to the session on each retry.
                    let thread = if flow_id_opt.is_some() {
                        Vec::new()
                    } else if let Some(ref cache_dir) = self.workspace_cache_dir {
                        let store = crate::runtime::chat_store::ChatStore::new(cache_dir);
                        store.load_history(&s_id)
                    } else {
                        match self.authority.get_or_create_session(
                            s_id.clone(),
                            space,
                            session_name.clone(),
                        ) {
                            Ok(t) => t,
                            Err(e) => {
                                warn!(%e, session_id = %s_id, "start_run: get_or_create_session failed");
                                Vec::new()
                            }
                        }
                    };
                    (Some(s_id), thread)
                } else {
                    (None, Vec::new())
                };

                // Auto-register the space if not already known to the authority.
                // The C++ host injects space_id from SpaceManager; the authority
                // requires an explicit upsert before it can track runs.
                let _ = self.authority.upsert_space(Space {
                    id: space,
                    name: space.to_string(),
                    compaction_threshold_pct: crate::agent_loop::DEFAULT_THRESHOLD_PCT,
                    compaction_recency_turns: crate::agent_loop::DEFAULT_RECENCY_TURNS,
                });
                match self.authority.start_run_with_session(
                    space,
                    None,
                    payload,
                    maybe_session_id.clone(),
                ) {
                    Ok(run_id) => {
                        info!(%run_id, "start_run: created run, setting up fan-out");
                        let sub_outcome = self.authority.subscribe(format!("run:{run_id}"));
                        let sub_id = sub_outcome.id;
                        let mut receiver = sub_outcome.receiver;
                        if let Some(sink) = self.sink.lock().clone() {
                            let task = tokio::spawn(async move {
                                while let Some(event) = receiver.recv().await {
                                    let kind = match &event.payload {
                                        crate::protocol::events::RuntimeEventPayload::RunStatus { status, .. } => format!("run_status:{status}"),
                                        crate::protocol::events::RuntimeEventPayload::Token { .. } => "token".into(),
                                        crate::protocol::events::RuntimeEventPayload::ThinkingToken { .. } => "thinking_token".into(),
                                        crate::protocol::events::RuntimeEventPayload::Trace { .. } => "trace".into(),
                                        crate::protocol::events::RuntimeEventPayload::Log { .. } => "log".into(),
                                        _ => "other".into(),
                                    };
                                    info!(%sub_id, %kind, "fan-out: sending event to transport");
                                    if sink
                                        .send(RuntimeToClient::Event {
                                            subscription: sub_id,
                                            event,
                                        })
                                        .await
                                        .is_err()
                                    {
                                        info!(%sub_id, "fan-out: sink closed, exiting");
                                        break;
                                    }
                                }
                            });
                            self.fanout.lock().insert(sub_id, task);
                        } else {
                            info!("start_run: no sink available, fan-out task NOT spawned");
                        }

                        // Build doc-submission channel shared across all
                        // agent invocations in this run.
                        let (doc_tx, mut doc_rx) =
                            tokio::sync::mpsc::channel::<DocumentSubmitted>(64);

                        let ar = self.agent_runner.clone();

                        // Optionally create a FlowRuntime + initial context
                        // when the request carries a `flow_id`.
                        let (entry_system_prompt, maybe_flow_ctx) = if let Some(ref fid) =
                            flow_id_opt
                        {
                            // Load the flow definition via the registry (task 9.1).
                            let flow_def_opt = match self
                                .services
                                .flow_registry
                                .load_flow_def(fid, &workspace_root)
                                .await
                            {
                                Ok(d) => Some(d),
                                Err(e) => {
                                    warn!(flow_id = %fid, error = %e, "start_run: failed to load flow definition");
                                    None
                                }
                            };

                            let (flow_rt, _is_new) = self
                                .services
                                .flow_registry
                                .get_or_create(&workspace_root)
                                .await;

                            let (flow_run_id, entry_contexts) = match flow_def_opt {
                                Some(ref flow_def) => {
                                    match flow_rt.start_run(flow_def, &initial_input).await {
                                        Ok((frid, ctxs)) => {
                                            info!(%run_id, flow_run_id = %frid, flow_id = %fid, "start_run: flow run created");
                                            // Register the chat session so the supervision task
                                            // can route human-review notifications back to the
                                            // originating chat session via spawn_chat_turn.
                                            if let Some(ref sid) = maybe_session_id {
                                                flow_rt.register_chat_session(&frid, sid.0.clone());
                                            }
                                            (frid, ctxs)
                                        }
                                        Err(e) => {
                                            warn!(flow_id = %fid, error = %e, "start_run: FlowRuntime::start_run failed");
                                            let _ = self.authority.fail_run(run_id, e.to_string());
                                            return ControlResponse::Err {
                                                error: ControlError::Internal {
                                                    message: e.to_string(),
                                                },
                                            };
                                        }
                                    }
                                }
                                None => {
                                    warn!(flow_id = %fid, "start_run: no flow definition available, running without flow context");
                                    (String::new(), vec![])
                                }
                            };

                            // The first entry context becomes the entry agent's system prompt.
                            let entry_sys = entry_contexts
                                .first()
                                .map(super::agent_runner::render_system_message);

                            let flow_ctx = RunContext {
                                space_id: space,
                                workspace_root: workspace_root.clone(),
                                flow_id: Some(fid.clone()),
                                flow_run_id: Some(flow_run_id.clone()),
                                flow_runtime: Some(flow_rt.clone()),
                                doc_tx: doc_tx.clone(),
                                llm_config: LlmConfig::from_payload_fields(
                                    &provider_kind,
                                    base_url.clone(),
                                    api_key.clone(),
                                    model.clone(),
                                ),
                                sandbox_tier: match &self.sandbox_policy {
                                    Some(p) => SandboxTier::Sandboxed(p.clone()),
                                    None => SandboxTier::Trusted,
                                },
                                workspace_cache_dir: self.workspace_cache_dir.clone(),
                            };
                            self.flow_contexts
                                .lock()
                                .insert(flow_run_id, flow_ctx.clone());

                            // Spawn ReactLoops for additional entry nodes (if any).
                            for ctx in entry_contexts.into_iter().skip(1) {
                                let agent_id = ctx.owner.clone();
                                ar.spawn_agent(flow_ctx.clone(), agent_id, ctx);
                            }

                            (entry_sys, Some(flow_ctx))
                        } else {
                            (None, None)
                        };

                        // Determine the effective system prompt (flow entry context
                        // overrides the plain system_prompt field).
                        let effective_system_prompt = entry_system_prompt.or(system_prompt);

                        // Supervision task: drains the DocumentSubmitted channel
                        // and calls FlowRuntime::on_document_submitted().
                        // When FlowRuntime returns downstream agents to invoke,
                        // it spawns new ReactLoops for them.
                        if let Some(ref flow_ctx) = maybe_flow_ctx {
                            let fctx = flow_ctx.clone();
                            let sup_services = Arc::clone(&self.services);
                            let sup_ar = ar.clone();
                            tokio::spawn(async move {
                                while let Some(evt) = doc_rx.recv().await {
                                    info!(
                                        run_id = %evt.run_id,
                                        agent_id = %evt.agent_id,
                                        doc_type = %evt.doc_type,
                                        document_id = %evt.document_id,
                                        "supervision: document submitted"
                                    );

                                    // Load flow definition via registry (task 9.1).
                                    let flow_def = match sup_services
                                        .flow_registry
                                        .load_flow_def(
                                            fctx.flow_id.as_deref().unwrap_or(""),
                                            &fctx.workspace_root,
                                        )
                                        .await
                                    {
                                        Ok(d) => d,
                                        Err(e) => {
                                            warn!(error = %e, "supervision: failed to load flow definition");
                                            continue;
                                        }
                                    };

                                    // Process the document submission.
                                    match fctx
                                        .flow_runtime
                                        .as_ref()
                                        .unwrap()
                                        .on_document_submitted(
                                            &evt.run_id,
                                            &evt.agent_id,
                                            &evt.doc_type,
                                            &evt.body,
                                            &flow_def,
                                            &evt.sha256,
                                            evt.revision,
                                        )
                                        .await
                                    {
                                        Ok(contexts) => {
                                            for inv_ctx in contexts {
                                                let agent_id = inv_ctx.owner.clone();
                                                if agent_id == "human" {
                                                    // Human review pending — notify the chat session.
                                                    if let Some(session_id) = fctx
                                                        .flow_runtime
                                                        .as_ref()
                                                        .unwrap()
                                                        .lookup_chat_session(
                                                            fctx.flow_run_id
                                                                .as_deref()
                                                                .unwrap_or(""),
                                                        )
                                                    {
                                                        let port = inv_ctx
                                                            .trigger
                                                            .approved_port
                                                            .as_deref()
                                                            .unwrap_or("?");
                                                        let producer = inv_ctx
                                                            .trigger
                                                            .from_node
                                                            .as_deref()
                                                            .unwrap_or("?");
                                                        let msg = format!(
                                                            "📋 **Review requested**: Node `{producer}` has submitted the document at port `{port}` for your review.\n\
                                                             Use `flow_get_pending_reviews` to list pending documents and `flow_approve` or `flow_request_changes` to respond."
                                                        );
                                                        sup_ar.spawn_chat(
                                                            fctx.clone(),
                                                            session_id,
                                                            msg,
                                                        );
                                                    }
                                                } else {
                                                    info!(
                                                        agent_id,
                                                        node_id = %inv_ctx.node_id,
                                                        "supervision: spawning downstream agent"
                                                    );
                                                    sup_ar.spawn_agent(
                                                        fctx.clone(),
                                                        agent_id,
                                                        inv_ctx,
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!(error = %e, run_id = %evt.run_id, "supervision: on_document_submitted failed");
                                            // Cycle limit exceeded or other terminal error — fail the run.
                                            if e.to_string().contains("cycle limit exceeded") {
                                                let _ = sup_services.authority.fail_run(
                                                    RunId(
                                                        Uuid::parse_str(&evt.run_id)
                                                            .unwrap_or_default(),
                                                    ),
                                                    e.to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                                info!("supervision: doc channel closed, task exiting");
                            });
                        } else {
                            // No flow context — drain the doc channel but do nothing with it.
                            tokio::spawn(async move {
                                while let Some(evt) = doc_rx.recv().await {
                                    info!(doc_type = %evt.doc_type, "supervision (no flow): document submitted");
                                }
                            });
                        }

                        // Build the HostCapabilityDispatcher for the entry agent.
                        let entry_agent_id = maybe_flow_ctx
                            .as_ref()
                            .map(|_c| {
                                // We need to reconstruct which agent was scheduled.
                                // For simplicity, pass a placeholder; the agent_id in
                                // submit_document will be populated from the context.
                                "entry-agent".to_owned()
                            })
                            .unwrap_or_else(|| "agent".to_owned());

                        let mut cap_builder = HostCapabilityDispatcher::builder();
                        cap_builder
                            .register_shell(Arc::new(LocalShell::new(&workspace_root)), false);
                        cap_builder.register_filesystem(
                            Arc::new(LocalFilesystem),
                            WorkspaceScope::new(&workspace_root),
                        );
                        cap_builder.register_notify(Arc::new(NullNotify));
                        if let Some(ref fid) = flow_id_opt {
                            let store = crate::capability::test_runner::LastReportStore::new();
                            cap_builder.register_test_runner(
                                workspace_root.clone(),
                                store,
                                run_id.to_string(),
                                "producer",
                            );
                            let flow_run_id_for_tool = maybe_flow_ctx
                                .as_ref()
                                .map(|c| {
                                    c.flow_run_id.clone().unwrap_or_else(|| run_id.to_string())
                                })
                                .unwrap_or_else(|| run_id.to_string());
                            cap_builder.register_submit_document(
                                workspace_root.clone(),
                                fid.clone(),
                                flow_run_id_for_tool,
                                entry_agent_id,
                                doc_tx.clone(),
                            );
                        }
                        cap_builder.register_search(workspace_root.clone());
                        cap_builder.register_git(workspace_root.clone());

                        // Register flow.* tools for the chat session when a flow context
                        // is available. The session id (if any) is registered so human-review
                        // notifications can route back to this session.
                        if let Some(ref flow_ctx) = maybe_flow_ctx {
                            let fctx_spawn = flow_ctx.clone();
                            let ar_spawn = ar.clone();
                            let spawn_fn: SpawnAgentFn =
                                Arc::new(move |_flow_run_id, agent_id, inv_ctx| {
                                    ar_spawn.spawn_agent(fctx_spawn.clone(), agent_id, inv_ctx);
                                });
                            register_flow_tools(
                                &mut cap_builder,
                                flow_ctx.flow_runtime.as_ref().unwrap().clone(),
                                workspace_root.clone(),
                                maybe_session_id
                                    .as_ref()
                                    .map(|s| s.0.clone())
                                    .unwrap_or_default(),
                                spawn_fn,
                            );
                        } else {
                            // Non-flow chat turn: register flow tools with the shared
                            // FlowRuntime so the user can list/start/approve flows.
                            let (shared_rt, is_new) = self
                                .services
                                .flow_registry
                                .get_or_create(&workspace_root)
                                .await;
                            let flow_rt_for_spawn = shared_rt.clone();
                            let flow_rt_for_tools = shared_rt.clone();
                            let authority_for_spawn = self.authority.clone();
                            let space_for_spawn = space;
                            let workspace_root_for_spawn = workspace_root.clone();
                            let base_url_for_spawn = base_url.clone();
                            let api_key_for_spawn = api_key.clone();
                            let model_for_spawn = model.clone();
                            let provider_kind_for_spawn = provider_kind.clone();
                            let sandbox_for_spawn = self.sandbox_policy.clone();
                            let wcd_for_spawn = self.workspace_cache_dir.clone();
                            let services_for_spawn = Arc::clone(&self.services);
                            let ar_for_spawn = ar.clone();
                            // Per-run doc_tx map: lazily create supervision task on first
                            // spawn for each flow_run_id.
                            let run_doc_txs: Arc<
                                Mutex<
                                    HashMap<String, tokio::sync::mpsc::Sender<DocumentSubmitted>>,
                                >,
                            > = Arc::new(Mutex::new(HashMap::new()));
                            let spawn_fn: SpawnAgentFn = Arc::new(
                                move |flow_run_id, agent_id, inv_ctx| {
                                    let doc_tx = {
                                        let mut map = run_doc_txs.lock();
                                        if let Some(tx) = map.get(&flow_run_id) {
                                            tx.clone()
                                        } else {
                                            let (tx, mut rx) =
                                                tokio::sync::mpsc::channel::<DocumentSubmitted>(64);
                                            map.insert(flow_run_id.clone(), tx.clone());
                                            // Start a supervision task for this flow run.
                                            let sup_flow_run_id = flow_run_id.clone();
                                            let sup_flow_rt = flow_rt_for_spawn.clone();
                                            let sup_authority = authority_for_spawn.clone();
                                            let sup_space = space_for_spawn;
                                            let sup_workspace_root =
                                                workspace_root_for_spawn.clone();
                                            let sup_base_url = base_url_for_spawn.clone();
                                            let sup_api_key = api_key_for_spawn.clone();
                                            let sup_model = model_for_spawn.clone();
                                            let sup_provider_kind = provider_kind_for_spawn.clone();
                                            let sup_sandbox = sandbox_for_spawn.clone();
                                            let sup_wcd = wcd_for_spawn.clone();
                                            let sup_services = services_for_spawn.clone();
                                            let sup_ar = ar_for_spawn.clone();
                                            let tx_clone = tx.clone();
                                            tokio::spawn(async move {
                                                while let Some(evt) = rx.recv().await {
                                                    info!(
                                                        run_id = %evt.run_id,
                                                        agent_id = %evt.agent_id,
                                                        doc_type = %evt.doc_type,
                                                        "supervision(chat): document submitted"
                                                    );
                                                    let flow_id = match sup_flow_rt
                                                        .get_run(&sup_flow_run_id)
                                                    {
                                                        Some(s) => s.flow_id.clone(),
                                                        None => {
                                                            warn!(run_id = %evt.run_id, "supervision(chat): run not found");
                                                            continue;
                                                        }
                                                    };
                                                    let flow_def = match sup_services
                                                        .flow_registry
                                                        .load_flow_def(
                                                            &flow_id,
                                                            &sup_workspace_root,
                                                        )
                                                        .await
                                                    {
                                                        Ok(d) => d,
                                                        Err(e) => {
                                                            warn!(error = %e, "supervision(chat): failed to load flow definition");
                                                            continue;
                                                        }
                                                    };
                                                    let fctx = RunContext {
                                                        space_id: sup_space,
                                                        workspace_root: sup_workspace_root.clone(),
                                                        flow_id: Some(flow_id),
                                                        flow_run_id: Some(sup_flow_run_id.clone()),
                                                        flow_runtime: Some(sup_flow_rt.clone()),
                                                        doc_tx: tx_clone.clone(),
                                                        llm_config: LlmConfig::from_payload_fields(
                                                            &sup_provider_kind,
                                                            sup_base_url.clone(),
                                                            sup_api_key.clone(),
                                                            sup_model.clone(),
                                                        ),
                                                        sandbox_tier: match &sup_sandbox {
                                                            Some(p) => {
                                                                SandboxTier::Sandboxed(p.clone())
                                                            }
                                                            None => SandboxTier::Trusted,
                                                        },
                                                        workspace_cache_dir: sup_wcd.clone(),
                                                    };
                                                    match sup_flow_rt
                                                        .on_document_submitted(
                                                            &evt.run_id,
                                                            &evt.agent_id,
                                                            &evt.doc_type,
                                                            &evt.body,
                                                            &flow_def,
                                                            &evt.sha256,
                                                            evt.revision,
                                                        )
                                                        .await
                                                    {
                                                        Ok(contexts) => {
                                                            for inv_ctx in contexts {
                                                                let next_agent =
                                                                    inv_ctx.owner.clone();
                                                                if next_agent == "human" {
                                                                    if let Some(sid) = sup_flow_rt
                                                                        .lookup_chat_session(
                                                                            &sup_flow_run_id,
                                                                        )
                                                                    {
                                                                        let port = inv_ctx
                                                                            .trigger
                                                                            .approved_port
                                                                            .as_deref()
                                                                            .unwrap_or("?");
                                                                        let producer = inv_ctx
                                                                            .trigger
                                                                            .from_node
                                                                            .as_deref()
                                                                            .unwrap_or("?");
                                                                        let msg = format!(
                                                                        "📋 **Review requested**: Node `{producer}` has submitted the document at port `{port}` for your review.\n\
                                                                         Use `flow_get_pending_reviews` to list pending documents and `flow_approve` or `flow_request_changes` to respond."
                                                                    );
                                                                        sup_ar.spawn_chat(
                                                                            fctx.clone(),
                                                                            sid,
                                                                            msg,
                                                                        );
                                                                    }
                                                                } else {
                                                                    sup_ar.spawn_agent(
                                                                        fctx.clone(),
                                                                        next_agent,
                                                                        inv_ctx,
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            warn!(error = %e, run_id = %evt.run_id, "supervision(chat): on_document_submitted failed");
                                                            if e.to_string()
                                                                .contains("cycle limit exceeded")
                                                            {
                                                                let _ = sup_authority.fail_run(
                                                                    RunId(
                                                                        Uuid::parse_str(
                                                                            &evt.run_id,
                                                                        )
                                                                        .unwrap_or_default(),
                                                                    ),
                                                                    e.to_string(),
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                                info!(flow_run_id = %sup_flow_run_id, "supervision(chat): doc channel closed");
                                            });
                                            tx
                                        }
                                    };
                                    let flow_id = match flow_rt_for_spawn.get_run(&flow_run_id) {
                                        Some(s) => s.flow_id.clone(),
                                        None => {
                                            warn!(%flow_run_id, "spawn_fn(chat): run not found");
                                            return;
                                        }
                                    };
                                    let fctx = RunContext {
                                        space_id: space_for_spawn,
                                        workspace_root: workspace_root_for_spawn.clone(),
                                        flow_id: Some(flow_id),
                                        flow_run_id: Some(flow_run_id.clone()),
                                        flow_runtime: Some(flow_rt_for_spawn.clone()),
                                        doc_tx,
                                        llm_config: LlmConfig::from_payload_fields(
                                            &provider_kind_for_spawn,
                                            base_url_for_spawn.clone(),
                                            api_key_for_spawn.clone(),
                                            model_for_spawn.clone(),
                                        ),
                                        sandbox_tier: match &sandbox_for_spawn {
                                            Some(p) => SandboxTier::Sandboxed(p.clone()),
                                            None => SandboxTier::Trusted,
                                        },
                                        workspace_cache_dir: wcd_for_spawn.clone(),
                                    };
                                    ar_for_spawn.spawn_agent(fctx.clone(), agent_id, inv_ctx);
                                },
                            );
                            register_flow_tools(
                                &mut cap_builder,
                                flow_rt_for_tools,
                                workspace_root.clone(),
                                maybe_session_id
                                    .as_ref()
                                    .map(|s| s.0.clone())
                                    .unwrap_or_default(),
                                spawn_fn,
                            );

                            // On first access after app restart, notify the session
                            // about any paused runs that have pending human reviews.
                            if is_new {
                                if let Some(ref sid) = maybe_session_id {
                                    let paused_runs: Vec<_> = shared_rt
                                        .list_runs()
                                        .into_iter()
                                        .filter(|r| {
                                            r.status == crate::flow::runtime::FlowRunStatus::Paused
                                        })
                                        .filter(|r| r.originating_session_id.is_some())
                                        .filter(|r| {
                                            r.node_states.values().any(|ns| {
                                                ns.ports.values().any(|&s| {
                                                    s == crate::flow::runtime::PortStatus::InReview
                                                })
                                            })
                                        })
                                        .collect();

                                    for run_state in paused_runs {
                                        let (ntx, _) =
                                            tokio::sync::mpsc::channel::<DocumentSubmitted>(1);
                                        let notify_fctx = RunContext {
                                            space_id: space,
                                            workspace_root: workspace_root.clone(),
                                            flow_id: Some(run_state.flow_id.clone()),
                                            flow_run_id: Some(run_state.run_id.clone()),
                                            flow_runtime: Some(shared_rt.clone()),
                                            doc_tx: ntx,
                                            llm_config: LlmConfig::from_payload_fields(
                                                &provider_kind,
                                                base_url.clone(),
                                                api_key.clone(),
                                                model.clone(),
                                            ),
                                            sandbox_tier: match &self.sandbox_policy {
                                                Some(p) => SandboxTier::Sandboxed(p.clone()),
                                                None => SandboxTier::Trusted,
                                            },
                                            workspace_cache_dir: self.workspace_cache_dir.clone(),
                                        };
                                        let msg = format!(
                                            "📋 **Pending review** (restored after restart): Flow run `{}` (flow: `{}`) has documents awaiting your review.\n\
                                             Use `flow_get_pending_reviews` to list them and `flow_approve` or `flow_request_changes` to respond.",
                                            run_state.run_id, run_state.flow_id
                                        );
                                        self.agent_runner.spawn_chat(
                                            notify_fctx,
                                            sid.0.clone(),
                                            msg,
                                        );
                                    }
                                }
                            }
                        }

                        // For direct-chat (no flow), use pre-loaded Chat.agent.yaml to get tool
                        // allow-list and inject_workspace preference. Flow paths handled
                        // per-agent inside spawn_agent_loop.
                        let chat_agent_def = preloaded_chat_agent_def;

                        if let Some(ref def) = chat_agent_def {
                            if !def.tools.is_empty() {
                                cap_builder.set_allowed_tools(def.tools.clone());
                            }
                        }

                        let tools = Arc::new(cap_builder.build());

                        // Build effective system prompt for direct-chat: Chat.agent.yaml
                        // system_prompt + workspace injection block (if opted in).
                        let effective_system_prompt = if let Some(ref def) = chat_agent_def {
                            let base = effective_system_prompt.or_else(|| {
                                if def.system_prompt.is_empty() {
                                    None
                                } else {
                                    Some(def.system_prompt.clone())
                                }
                            });
                            if def.inject_workspace {
                                let defs = tools.definitions();
                                let mut tool_names_sorted: Vec<&str> =
                                    defs.iter().map(|d| d.name.as_str()).collect();
                                tool_names_sorted.sort_unstable();
                                let block = build_workspace_injection_block(
                                    &workspace_root,
                                    &tool_names_sorted,
                                );
                                Some(format!("{}{block}", base.unwrap_or_default()))
                            } else {
                                base
                            }
                        } else {
                            effective_system_prompt
                        };
                        // Render prompt variables (task 5.6).
                        let agent_name = chat_agent_def
                            .as_ref()
                            .map(|d| d.name.clone())
                            .unwrap_or_default();
                        let agent_vars = chat_agent_def
                            .as_ref()
                            .map(|d| d.vars.clone())
                            .unwrap_or_default();
                        // Load workspace agent names for the ${agents} variable (Crony orchestration).
                        let workspace_agent_names: Vec<String> = {
                            use crate::workspace::{AgentRegistry, Workspace};
                            let layout = Workspace::new(&workspace_root);
                            let mut reg = AgentRegistry::new(layout.agents_dir());
                            reg.refresh().await;
                            reg.entries()
                                .into_iter()
                                .map(|(name, desc)| {
                                    if desc.is_empty() {
                                        name
                                    } else {
                                        format!("- **{name}** — {desc}")
                                    }
                                })
                                .collect()
                        };
                        let effective_system_prompt = effective_system_prompt.map(|tmpl| {
                            let prompt_ctx = super::prompt::VarContext::builder()
                                .workspace_root(workspace_root.clone())
                                .agent_name(agent_name.clone())
                                .user_vars(agent_vars)
                                .agents(workspace_agent_names)
                                .build();
                            super::prompt::render(&tmpl, &prompt_ctx)
                        });

                        let authority = self.authority.clone();
                        let memory_manager = self.services.memory_manager.clone();
                        let workspace_cache_dir_clone = self.workspace_cache_dir.clone();
                        let is_copilot = provider_kind == "github_copilot";
                        let is_anthropic = provider_kind == "anthropic";
                        let chat_reasoning_effort = reasoning_effort.clone();
                        let chat_anthropic_effort = anthropic_effort.clone();
                        tokio::spawn(async move {
                            // For GitHub Copilot, exchange the stored GitHub OAuth token for
                            // the short-lived Copilot API token required by the API.
                            let (effective_api_key, copilot_mode) = if is_copilot {
                                match api_key.as_deref() {
                                    Some(github_token) if !github_token.is_empty() => {
                                        let http = reqwest::Client::builder()
                                            .timeout(std::time::Duration::from_secs(30))
                                            .build()
                                            .unwrap_or_default();
                                        match copilot_auth::exchange_for_copilot_token(
                                            &http,
                                            github_token,
                                        )
                                        .await
                                        {
                                            Ok(ct) => {
                                                info!(%run_id, "react_loop: copilot token exchanged successfully");
                                                (Some(ct.token), true)
                                            }
                                            Err(e) => {
                                                warn!(%run_id, error = %e, "react_loop: copilot token exchange failed, using raw token");
                                                (api_key, true)
                                            }
                                        }
                                    }
                                    _ => (api_key, true),
                                }
                            } else {
                                (api_key, false)
                            };

                            // Probe model capabilities for thinking support.
                            // Only probe when using the native Anthropic provider — other
                            // providers (OpenAI-compat, Copilot, …) don't accept the
                            // Anthropic-style `thinking` field and would reject the request.
                            let thinking_config = if is_anthropic {
                                let caps = CapabilityResolver::resolve(
                                    &model,
                                    &base_url,
                                    effective_api_key.as_deref(),
                                )
                                .await;
                                apply_anthropic_effort_override(
                                    caps.thinking_config(),
                                    chat_anthropic_effort.as_deref(),
                                )
                            } else {
                                None
                            };

                            info!(%run_id, llm_base_url = %base_url, %model, "react_loop: starting");
                            let llm: Arc<dyn crate::llm::LlmProvider> = if is_anthropic {
                                let cfg = AnthropicConfig {
                                    base_url: base_url.clone(),
                                    api_key: effective_api_key,
                                    default_model: model.clone(),
                                    ..Default::default()
                                };
                                match AnthropicProvider::new(cfg) {
                                    Ok(p) => Arc::new(p),
                                    Err(e) => {
                                        info!(%run_id, error = %e, "react_loop: AnthropicProvider::new failed");
                                        let _ = authority.fail_run(run_id, e.to_string());
                                        return;
                                    }
                                }
                            } else {
                                let llm_cfg = OpenAiConfig {
                                    base_url: base_url.clone(),
                                    api_key: effective_api_key,
                                    default_model: model.clone(),
                                    copilot_mode,
                                    ..Default::default()
                                };
                                match OpenAiProvider::new(llm_cfg) {
                                    Ok(p) => Arc::new(p),
                                    Err(e) => {
                                        info!(%run_id, error = %e, "react_loop: OpenAiProvider::new failed");
                                        let _ = authority.fail_run(run_id, e.to_string());
                                        return;
                                    }
                                }
                            };
                            // Chat without a flow / authored system prompt
                            // gets a small default so the model has a clear
                            // "you may stop now" condition. Without this,
                            // gpt-4o-class models often keep calling tools
                            // until max_turns hits.
                            let chat_system_prompt = effective_system_prompt
                                .or_else(|| Some(default_chat_system_prompt()));

                            // Compact the session thread before starting the run
                            // if it is approaching the model's context limit.
                            let effective_thread = if !prior_thread.is_empty() {
                                let result = maybe_compact(
                                    prior_thread,
                                    llm.clone(),
                                    &model,
                                    DEFAULT_THRESHOLD_PCT,
                                    DEFAULT_RECENCY_TURNS,
                                )
                                .await;
                                if result.compacted {
                                    // Persist the compacted thread and optionally
                                    // write the summary to the session memory namespace.
                                    if let Some(ref sid) = maybe_session_id {
                                        let _ = authority.flush_thread(sid, result.thread.clone());
                                        if let Some(ref summary) = result.summary {
                                            let ns = crate::runtime::state::MemoryNamespaceId::from(
                                                format!("session:{sid}").as_str(),
                                            );
                                            let count = authority
                                                .session_thread(sid)
                                                .map(|t| t.len())
                                                .unwrap_or(0);
                                            let mem_key = format!("compaction/{count}");
                                            let _ = authority.put_memory(ns.clone(), crate::runtime::state::MemoryEntry {
                                                key: mem_key.clone(),
                                                value: serde_json::json!({ "summary": summary }),
                                                updated_at_ms: crate::runtime::authority::now_ms(),
                                            });
                                            // Emit a trace so the UI can show memory-write events.
                                            authority.emit_for_run(
                                                run_id,
                                                crate::protocol::events::RuntimeEventPayload::Trace {
                                                    run_id: run_id.to_string(),
                                                    trace: serde_json::json!({
                                                        "kind": "memory_write",
                                                        "namespace": ns.0,
                                                        "key": mem_key,
                                                        "source": "compaction",
                                                    }),
                                                },
                                            );
                                        }
                                    }
                                }
                                result.thread
                            } else {
                                prior_thread
                            };

                            let cfg = LoopConfig {
                                model,
                                system_prompt: chat_system_prompt,
                                user_input,
                                max_turns: 99999,
                                temperature: None,
                                reasoning_effort: chat_reasoning_effort,
                                llm,
                                tools,
                                thinking: thinking_config,
                                initial_thread: if effective_thread.is_empty() {
                                    None
                                } else {
                                    Some(effective_thread)
                                },
                                // Flow runs must not flush their temporary PM-agent
                                // history back to the chat session. Using None here
                                // prevents ReactLoop from calling flush_thread and
                                // contaminating the session thread with flow messages.
                                session_id: if maybe_flow_ctx.is_some() {
                                    None
                                } else {
                                    maybe_session_id.clone()
                                },
                                reflection: chat_agent_def
                                    .as_ref()
                                    .and_then(|d| d.reflection.clone()),
                                write_namespace: chat_agent_def
                                    .as_ref()
                                    .filter(|d| !d.memory_namespace.is_empty())
                                    .map(|d| {
                                        crate::runtime::state::MemoryNamespaceId::from(
                                            d.memory_namespace.as_str(),
                                        )
                                    }),
                                memory_manager: memory_manager.clone(),
                                middleware: build_middleware_chain(authority.clone()),
                            };
                            let result = ReactLoop::new(authority.clone(), run_id, cfg).run().await;
                            info!(%run_id, ok = result.is_ok(), "react_loop: finished");
                            if let Err(e) = &result {
                                info!(%run_id, error = %e, "react_loop: failed with error");
                            }
                            // If a ChatStore is configured, append the final session thread
                            // to history.jsonl. The ReactLoop already flushed to the authority
                            // snapshot internally; we read it back here to write the JSONL copy.
                            // Flow runs must NOT write back to the chat session history — doing
                            // so accumulates PM-agent messages across retries, which corrupts
                            // the role-alternation sequence and causes subsequent LLM 400 errors.
                            if maybe_flow_ctx.is_none() {
                                if let (Some(ref sid), Some(ref cache_dir)) =
                                    (&maybe_session_id, &workspace_cache_dir_clone)
                                {
                                    if let Some(thread) = authority.session_thread(sid) {
                                        if !thread.is_empty() {
                                            let store = crate::runtime::chat_store::ChatStore::new(
                                                cache_dir,
                                            );
                                            let _ = store.append_turns(sid, &thread);
                                        }
                                    }
                                }
                            }
                        });
                        ControlResponse::RunStarted {
                            run_id: run_id.to_string(),
                            subscription: sub_id,
                        }
                    }
                    Err(e) => ControlResponse::Err {
                        error: authority_err_to_control(e, Some(&space_id), None),
                    },
                }
            }

            ControlRequest::CancelRun { run_id } => self.run_op(&run_id, |a, id| a.cancel_run(id)),
            ControlRequest::PauseRun { run_id } => self.run_op(&run_id, |a, id| a.pause_run(id)),
            ControlRequest::ResumeRun { run_id } => {
                // ── Parse and validate ────────────────────────────────────
                let id = match parse_run(&run_id) {
                    Ok(r) => r,
                    Err(resp) => return resp,
                };

                // Fetch the persisted Run so we can reconstruct the context.
                let run = {
                    let snap = self.authority.snapshot();
                    match snap.runs.get(&id).cloned() {
                        Some(r) => r,
                        None => {
                            return ControlResponse::Err {
                                error: ControlError::UnknownRun {
                                    run_id: run_id.clone(),
                                },
                            }
                        }
                    }
                };

                // Only resume Paused or AwaitingReview runs.
                use crate::runtime::state::RunStatus;
                match &run.status {
                    RunStatus::Paused | RunStatus::AwaitingReview => {}
                    _ => return ControlResponse::Ack, // already running or terminal
                }

                // ── Reconstruct startup context from the persisted spec ───
                let spec = &run.spec;
                let llm_obj = spec.get("llm");
                let base_url = llm_obj
                    .and_then(|l| l.get("base_url"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("https://api.openai.com/v1")
                    .to_string();
                let api_key = llm_obj
                    .and_then(|l| l.get("api_key"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let model = llm_obj
                    .and_then(|l| l.get("model"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("gpt-4o-mini")
                    .to_string();
                let provider_kind = llm_obj
                    .and_then(|l| l.get("provider_kind"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("openai_compat")
                    .to_string();
                let user_input = spec
                    .get("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let workspace_root: PathBuf = spec
                    .get("workspace_root")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| self.workspace_roots.first().cloned())
                    .unwrap_or_else(std::env::temp_dir);

                let maybe_session_id = run.session_id.clone();

                // ── Load prior conversation thread ────────────────────────
                // The ChatStore holds the authoritative conversation history
                // for the session; fall back to the in-memory snapshot thread
                // when no ChatStore is configured.
                let prior_thread = if let Some(ref sid) = maybe_session_id {
                    if let Some(ref cache_dir) = self.workspace_cache_dir {
                        let store = crate::runtime::chat_store::ChatStore::new(cache_dir);
                        store.load_history(sid)
                    } else {
                        self.authority.session_thread(sid).unwrap_or_default()
                    }
                } else {
                    Vec::new()
                };

                // ── Transition the run back to Running ───────────────────
                if let Err(e) = self.authority.mark_run_running(id) {
                    return ControlResponse::Err {
                        error: authority_err_to_control(e, None, Some(&run_id)),
                    };
                }
                info!(%id, "resume_run: run marked running, spawning agent loop");

                // ── Re-attach fan-out subscription ────────────────────────
                let sub_outcome = self.authority.subscribe(format!("run:{id}"));
                let sub_id = sub_outcome.id;
                let mut receiver = sub_outcome.receiver;
                if let Some(sink) = self.sink.lock().clone() {
                    let task = tokio::spawn(async move {
                        while let Some(event) = receiver.recv().await {
                            if sink
                                .send(RuntimeToClient::Event {
                                    subscription: sub_id,
                                    event,
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                    self.fanout.lock().insert(sub_id, task);
                }

                // ── Build capability dispatcher ───────────────────────────
                let mut cap_builder = HostCapabilityDispatcher::builder();
                cap_builder.register_shell(Arc::new(LocalShell::new(&workspace_root)), false);
                cap_builder.register_filesystem(
                    Arc::new(LocalFilesystem),
                    WorkspaceScope::new(&workspace_root),
                );
                cap_builder.register_notify(Arc::new(NullNotify));
                cap_builder.register_search(workspace_root.clone());
                cap_builder.register_git(workspace_root.clone());

                let resolved_agent_id = run
                    .agent_id
                    .as_ref()
                    .map(|a| a.to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| crate::crony::CronyBuiltin::ID.to_owned());
                let workspace_root_clone = workspace_root.clone();
                let chat_agent_def = agent_loader::load_agent_with_builtin(
                    &workspace_root_clone,
                    &resolved_agent_id,
                )
                .await;
                if !chat_agent_def.tools.is_empty() {
                    cap_builder.set_allowed_tools(chat_agent_def.tools.clone());
                }
                let tools = Arc::new(cap_builder.build());

                // ── Build effective system prompt ─────────────────────────
                let base_system_prompt = spec
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| {
                        if chat_agent_def.system_prompt.is_empty() {
                            None
                        } else {
                            Some(chat_agent_def.system_prompt.clone())
                        }
                    });
                let effective_system_prompt = if chat_agent_def.inject_workspace {
                    let defs = tools.definitions();
                    let mut tool_names_sorted: Vec<&str> =
                        defs.iter().map(|d| d.name.as_str()).collect();
                    tool_names_sorted.sort_unstable();
                    let block =
                        build_workspace_injection_block(&workspace_root, &tool_names_sorted);
                    Some(format!("{}{block}", base_system_prompt.unwrap_or_default()))
                } else {
                    base_system_prompt
                };

                // ── Spawn the agent loop task ─────────────────────────────
                let authority = self.authority.clone();
                let memory_manager = self.services.memory_manager.clone();
                let workspace_cache_dir_clone = self.workspace_cache_dir.clone();
                let is_copilot = provider_kind == "github_copilot";
                let is_anthropic = provider_kind == "anthropic";
                // Resumed runs don't have per-message reasoning_effort plumbed
                // through the persisted run state yet; fall back to the
                // provider/agent defaults applied by the loop config.
                let chat_reasoning_effort: Option<String> = None;

                tokio::spawn(async move {
                    let (effective_api_key, copilot_mode) = if is_copilot {
                        match api_key.as_deref() {
                            Some(github_token) if !github_token.is_empty() => {
                                let http = reqwest::Client::builder()
                                    .timeout(std::time::Duration::from_secs(30))
                                    .build()
                                    .unwrap_or_default();
                                match copilot_auth::exchange_for_copilot_token(&http, github_token)
                                    .await
                                {
                                    Ok(ct) => (Some(ct.token), true),
                                    Err(_) => (api_key, true),
                                }
                            }
                            _ => (api_key, true),
                        }
                    } else {
                        (api_key, false)
                    };

                    let thinking_config = if is_anthropic {
                        let caps = CapabilityResolver::resolve(
                            &model,
                            &base_url,
                            effective_api_key.as_deref(),
                        )
                        .await;
                        caps.thinking_config()
                    } else {
                        None
                    };

                    info!(%id, llm_base_url = %base_url, %model, "resume_run: react_loop starting");
                    let llm: Arc<dyn crate::llm::LlmProvider> = if is_anthropic {
                        let cfg = AnthropicConfig {
                            base_url: base_url.clone(),
                            api_key: effective_api_key,
                            default_model: model.clone(),
                            ..Default::default()
                        };
                        match AnthropicProvider::new(cfg) {
                            Ok(p) => Arc::new(p),
                            Err(e) => {
                                let _ = authority.fail_run(id, e.to_string());
                                return;
                            }
                        }
                    } else {
                        let llm_cfg = OpenAiConfig {
                            base_url: base_url.clone(),
                            api_key: effective_api_key,
                            default_model: model.clone(),
                            copilot_mode,
                            ..Default::default()
                        };
                        match OpenAiProvider::new(llm_cfg) {
                            Ok(p) => Arc::new(p),
                            Err(e) => {
                                let _ = authority.fail_run(id, e.to_string());
                                return;
                            }
                        }
                    };

                    // Compact the thread if it is approaching context limits.
                    let effective_thread = if !prior_thread.is_empty() {
                        let result = maybe_compact(
                            prior_thread,
                            llm.clone(),
                            &model,
                            DEFAULT_THRESHOLD_PCT,
                            DEFAULT_RECENCY_TURNS,
                        )
                        .await;
                        if result.compacted {
                            if let Some(ref sid) = maybe_session_id {
                                let _ = authority.flush_thread(sid, result.thread.clone());
                            }
                        }
                        result.thread
                    } else {
                        prior_thread
                    };

                    let cfg = LoopConfig {
                        model,
                        system_prompt: effective_system_prompt,
                        user_input,
                        max_turns: 99999,
                        temperature: None,
                        reasoning_effort: chat_reasoning_effort.clone(),
                        llm,
                        tools,
                        thinking: thinking_config,
                        initial_thread: if effective_thread.is_empty() {
                            None
                        } else {
                            Some(effective_thread)
                        },
                        session_id: maybe_session_id.clone(),
                        reflection: None,
                        write_namespace: None,
                        memory_manager: memory_manager.clone(),
                        middleware: build_middleware_chain(authority.clone()),
                    };

                    let result = ReactLoop::new(authority.clone(), id, cfg).run().await;
                    info!(%id, ok = result.is_ok(), "resume_run: react_loop finished");

                    if let (Some(ref sid), Some(ref cache_dir)) =
                        (&maybe_session_id, &workspace_cache_dir_clone)
                    {
                        if let Some(thread) = authority.session_thread(sid) {
                            if !thread.is_empty() {
                                let store = crate::runtime::chat_store::ChatStore::new(cache_dir);
                                let _ = store.append_turns(sid, &thread);
                            }
                        }
                    }
                });

                ControlResponse::RunStarted {
                    run_id: run_id.clone(),
                    subscription: sub_id,
                }
            }
            ControlRequest::SwapMemory {
                session_id,
                target,
                namespace_id,
            } => {
                // Validate target field.
                match target.as_str() {
                    "read" | "write" | "both" => {}
                    _ => {
                        return ControlResponse::Err {
                            error: ControlError::InvalidRequest {
                                message: format!(
                                    "SwapMemory: unknown target '{}', expected read|write|both",
                                    target
                                ),
                            },
                        };
                    }
                }
                let sid = crate::runtime::state::SessionId::from(session_id.as_str());
                let ns_id = crate::runtime::state::MemoryNamespaceId::from(namespace_id.as_str());
                if self
                    .authority
                    .update_session_namespaces(&sid, &target, ns_id)
                {
                    info!(
                        session_id,
                        target, namespace_id, "SwapMemory: namespace updated"
                    );
                    ControlResponse::Ack
                } else {
                    ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("SwapMemory: session '{}' not found", session_id),
                        },
                    }
                }
            }
            ControlRequest::PostInput { run_id, payload } => {
                self.run_op(&run_id, |a, id| a.post_input(id, payload.clone()))
            }
            ControlRequest::ResolveReview {
                run_id,
                review_id,
                decision,
                notes,
            } => {
                let run = match parse_run(&run_id) {
                    Ok(r) => r,
                    Err(resp) => return resp,
                };
                let review = match parse_review(&review_id) {
                    Ok(r) => r,
                    Err(resp) => return resp,
                };
                let perm_decision = match decision {
                    ReviewDecision::Approve => PermissionState::Approved,
                    ReviewDecision::Reject => PermissionState::Rejected,
                    ReviewDecision::Defer => PermissionState::Deferred,
                };

                // Wire to FlowRuntime if a flow context is registered for
                // this run. The `run_id` in `ResolveReview` is the
                // `flow_run_id` from FlowRuntime (returned in RunStarted).
                // We look up by flow_run_id and, if found, dispatch to the
                // appropriate FlowRuntime method.
                let flow_ctx_opt: Option<RunContext> = {
                    let map = self.flow_contexts.lock();
                    // Try a direct lookup by run_id string.
                    map.values()
                        .find(|c| {
                            c.flow_run_id.as_deref() == Some(run_id.as_str())
                                || c.flow_run_id.as_deref() == Some(run.to_string().as_str())
                        })
                        .cloned()
                };

                if let Some(fctx) = flow_ctx_opt {
                    // Spawn async work for the FlowRuntime call since
                    // handle_control is async but we don't want to block.
                    let flow_run_id = fctx.flow_run_id.clone().unwrap_or_default();
                    let is_approve = perm_decision == PermissionState::Approved;
                    let rr_services = Arc::clone(&self.services);
                    let rr_ar = self.agent_runner.clone();
                    tokio::spawn(async move {
                        let flow_def = match rr_services
                            .flow_registry
                            .load_flow_def(
                                fctx.flow_id.as_deref().unwrap_or(""),
                                &fctx.workspace_root,
                            )
                            .await
                        {
                            Ok(d) => d,
                            Err(e) => {
                                warn!(error = %e, "resolve_review: failed to load flow definition");
                                return;
                            }
                        };

                        // `review_id` encodes "<producing_agent>:<port>" when
                        // submitted via the flow pipeline. Try to parse it;
                        // fall back to a no-op if the format doesn't match.
                        let review_str = review_id.clone();
                        let parts: Vec<&str> = review_str.splitn(2, ':').collect();
                        if parts.len() != 2 {
                            info!(review_id = %review_str, "resolve_review: not a flow review_id, skipping FlowRuntime dispatch");
                            return;
                        }
                        let producing_agent = parts[0];
                        let port = parts[1];

                        if is_approve {
                            match fctx
                                .flow_runtime
                                .as_ref()
                                .unwrap()
                                .on_document_approved(
                                    &flow_run_id,
                                    producing_agent,
                                    port,
                                    &flow_def,
                                )
                                .await
                            {
                                Ok(contexts) => {
                                    for inv_ctx in contexts {
                                        let agent_id = inv_ctx.owner.clone();
                                        info!(agent_id, node_id = %inv_ctx.node_id, "resolve_review: scheduling after approval");
                                        rr_ar.spawn_agent(fctx.clone(), agent_id, inv_ctx);
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, "resolve_review: on_document_approved failed");
                                }
                            }
                        } else {
                            match fctx
                                .flow_runtime
                                .as_ref()
                                .unwrap()
                                .on_rejected_requeue(&flow_run_id, producing_agent, port, &flow_def)
                                .await
                            {
                                Ok(Some(inv_ctx)) => {
                                    let agent_id = inv_ctx.owner.clone();
                                    info!(agent_id, node_id = %inv_ctx.node_id, "resolve_review: requeueing after rejection");
                                    rr_ar.spawn_agent(fctx.clone(), agent_id, inv_ctx);
                                }
                                Ok(None) => {
                                    info!(
                                        producing_agent,
                                        port, "resolve_review: rejection, no requeue needed"
                                    );
                                }
                                Err(e) => {
                                    warn!(error = %e, "resolve_review: on_rejected_requeue failed");
                                }
                            }
                        }
                    });
                }

                // Always also resolve via the RuntimeAuthority (for legacy
                // non-flow runs and review-gate enforcement in the agent loop).
                match self
                    .authority
                    .resolve_review(run, review, perm_decision, notes)
                {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: authority_err_to_control(e, None, Some(&run_id)),
                    },
                }
            }

            // ── Phase 2: Workspace / file / flow handlers ─────────────────
            ControlRequest::WorkspaceLayout { workspace_root } => {
                use crate::workspace::Workspace;
                let layout = Workspace::new(&workspace_root);
                let version = layout.read_version().await;
                ControlResponse::Data {
                    payload: serde_json::json!({
                        "root":           layout.root().to_string_lossy(),
                        "cronymax_dir":   layout.cronymax_dir().to_string_lossy(),
                        "flows_dir":      layout.flows_dir().to_string_lossy(),
                        "agents_dir":     layout.agents_dir().to_string_lossy(),
                        "doc_types_dir":  layout.doc_types_dir().to_string_lossy(),
                        "conflicts_dir":  layout.conflicts_dir().to_string_lossy(),
                        "version":        version,
                        "layout_version": crate::workspace::Workspace::LAYOUT_VERSION,
                    }),
                }
            }

            ControlRequest::FileRead {
                workspace_root,
                path,
            } => {
                use crate::workspace::FileBroker;
                let broker = FileBroker::new(&workspace_root);
                match broker.read_text(std::path::Path::new(&path)).await {
                    Ok(content) => ControlResponse::Data {
                        payload: serde_json::json!({ "content": content }),
                    },
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            ControlRequest::FileWrite {
                workspace_root,
                path,
                content,
            } => {
                use crate::workspace::FileBroker;
                let broker = FileBroker::new(&workspace_root);
                match broker
                    .write_text(std::path::Path::new(&path), &content)
                    .await
                {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            ControlRequest::FlowList {
                workspace_root,
                builtin_flows_dir,
            } => {
                use crate::workspace::{load_flow_yaml, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut flows: Vec<serde_json::Value> = Vec::new();
                let mut local_ids = std::collections::HashSet::new();

                // Helper: build a flow summary entry from a parsed doc.
                fn flow_summary(
                    doc: &crate::workspace::FlowYamlDoc,
                    builtin: bool,
                ) -> serde_json::Value {
                    let agents: Vec<&str> = doc.agents.iter().map(|a| a.id.as_str()).collect();
                    let node_count = if doc.nodes.is_empty() {
                        doc.edges.len()
                    } else {
                        doc.nodes.len()
                    };
                    serde_json::json!({
                        "id": doc.id,
                        "name": doc.name,
                        "node_count": node_count,
                        // kept for back-compat with older frontends
                        "edge_count": doc.edges.len(),
                        "agents": agents,
                        "builtin": builtin,
                    })
                }

                // Scan workspace-local flows first.
                if let Ok(mut rd) = tokio::fs::read_dir(layout.flows_dir()).await {
                    while let Ok(Some(entry)) = rd.next_entry().await {
                        if !entry.path().is_dir() {
                            continue;
                        }
                        let id = entry.file_name().to_string_lossy().to_string();
                        let flow_yaml_path = entry.path().join("flow.yaml");
                        if let Some(doc) = load_flow_yaml(&flow_yaml_path, &id).await {
                            flows.push(flow_summary(&doc, false));
                            local_ids.insert(id);
                        }
                    }
                }

                // Merge builtin flows (dedup by id, workspace wins).
                if let Some(builtin_dir) = builtin_flows_dir {
                    if let Ok(mut rd) = tokio::fs::read_dir(&builtin_dir).await {
                        while let Ok(Some(entry)) = rd.next_entry().await {
                            if !entry.path().is_dir() {
                                continue;
                            }
                            let id = entry.file_name().to_string_lossy().to_string();
                            if local_ids.contains(&id) {
                                continue;
                            }
                            let flow_yaml_path = entry.path().join("flow.yaml");
                            if let Some(doc) = load_flow_yaml(&flow_yaml_path, &id).await {
                                flows.push(flow_summary(&doc, true));
                            }
                        }
                    }
                }

                ControlResponse::Data {
                    payload: serde_json::json!({ "flows": flows }),
                }
            }

            ControlRequest::FlowLoad {
                workspace_root,
                flow_id,
            } => {
                use crate::workspace::{load_flow_yaml, Workspace};
                let layout = Workspace::new(&workspace_root);
                let path = layout.flow_file(&flow_id);
                match load_flow_yaml(&path, &flow_id).await {
                    Some(doc) => {
                        let agents: Vec<&str> = doc.agents.iter().map(|a| a.id.as_str()).collect();
                        let edges: Vec<serde_json::Value> = doc
                            .edges
                            .iter()
                            .map(|e| {
                                serde_json::json!({
                                    "from": e.from,
                                    "to": e.to,
                                    "port": e.port,
                                    "requires_human_approval": e.requires_human_approval,
                                    "on_approved_reschedule": e.on_approved_reschedule,
                                    "reviewer_agents": e.reviewer_agents,
                                    "max_cycles": e.max_cycles,
                                    "on_cycle_exhausted": e.on_cycle_exhausted,
                                })
                            })
                            .collect();
                        let nodes: Vec<serde_json::Value> = doc
                            .nodes
                            .iter()
                            .map(|n| {
                                let outputs: Vec<serde_json::Value> = n
                                    .outputs
                                    .iter()
                                    .map(|o| {
                                        serde_json::json!({
                                            "port": o.port,
                                            "routes_to": o.routes_to,
                                            "reviewers": o.reviewers,
                                            "max_cycles": o.max_cycles,
                                            "on_cycle_exhausted": o.on_cycle_exhausted,
                                        })
                                    })
                                    .collect();
                                serde_json::json!({
                                    "id": n.id,
                                    "owner": n.owner,
                                    "outputs": outputs,
                                })
                            })
                            .collect();
                        ControlResponse::Data {
                            payload: serde_json::json!({
                                "id": doc.id,
                                "name": doc.name,
                                "description": doc.description,
                                "max_review_rounds": doc.max_review_rounds,
                                "on_review_exhausted": doc.on_review_exhausted,
                                "reviewer_enabled": doc.reviewer_enabled,
                                "reviewer_timeout_secs": doc.reviewer_timeout_secs,
                                "agents": agents,
                                "edges": edges,
                                "nodes": nodes,
                            }),
                        }
                    }
                    None => ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("flow not found: {flow_id}"),
                        },
                    },
                }
            }

            ControlRequest::FlowSave {
                workspace_root,
                flow_id,
                graph,
            } => {
                use crate::workspace::flows::flow_yaml_to_string;
                use crate::workspace::{FlowYamlDoc, FlowYamlEdge, Workspace};

                // Validate flow_id (alphanumeric + _ -)
                let valid = !flow_id.is_empty()
                    && flow_id.len() <= 64
                    && flow_id
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    && !flow_id.starts_with('-');
                if !valid {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("invalid flow_id: {flow_id}"),
                        },
                    };
                }

                // Build FlowYamlDoc from the graph payload.
                let mut agent_names: Vec<String> = Vec::new();
                let mut id_to_agent: std::collections::HashMap<i64, String> = Default::default();
                if let Some(nodes) = graph.get("nodes").and_then(|v| v.as_array()) {
                    for node in nodes {
                        let node_id = node.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
                        let agent_name = node
                            .get("config")
                            .and_then(|c| c.get("agent_name"))
                            .and_then(|v| v.as_str())
                            .or_else(|| node.get("name").and_then(|v| v.as_str()))
                            .unwrap_or("")
                            .to_owned();
                        if agent_name.is_empty() || node_id < 0 {
                            continue;
                        }
                        agent_names.push(agent_name.clone());
                        id_to_agent.insert(node_id, agent_name);
                    }
                }

                let mut edges: Vec<FlowYamlEdge> = Vec::new();
                if let Some(edge_arr) = graph.get("edges").and_then(|v| v.as_array()) {
                    for e in edge_arr {
                        let from_id = e.get("from_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                        let to_id = e.get("to_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                        let Some(from_agent) = id_to_agent.get(&from_id) else {
                            continue;
                        };
                        let to_agent = id_to_agent.get(&to_id).cloned().unwrap_or_default();
                        edges.push(FlowYamlEdge {
                            from: from_agent.clone(),
                            to: to_agent,
                            port: e
                                .get("port")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            requires_human_approval: e
                                .get("requires_human_approval")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            ..Default::default()
                        });
                    }
                }

                let doc = FlowYamlDoc {
                    id: flow_id.clone(),
                    name: flow_id.clone(),
                    agents: agent_names
                        .into_iter()
                        .map(|s| crate::workspace::flows::FlowYamlAgent { id: s })
                        .collect(),
                    edges,
                    ..Default::default()
                };

                let yaml = flow_yaml_to_string(&doc);
                let layout = Workspace::new(&workspace_root);
                let flow_path = layout.flow_file(&flow_id);
                if let Some(parent) = flow_path.parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        return ControlResponse::Err {
                            error: ControlError::Internal {
                                message: e.to_string(),
                            },
                        };
                    }
                }
                // Atomic write via temp file + rename.
                let tmp = flow_path.with_extension("yaml.tmp");
                if let Err(e) = tokio::fs::write(&tmp, &yaml).await {
                    return ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    };
                }
                if let Err(e) = tokio::fs::rename(&tmp, &flow_path).await {
                    return ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    };
                }
                ControlResponse::Ack
            }

            // ── Phase 3: Agent registry ───────────────────────────────────
            ControlRequest::AgentRegistryList { workspace_root } => {
                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                // Prepend the Crony builtin sentinel so the frontend always
                // sees it, even when no YAML file exists on disk.
                let mut agents: Vec<serde_json::Value> = vec![serde_json::json!({
                    "name": crate::crony::CronyBuiltin::ID,
                    "kind": "worker",
                    "llm": "",
                    "llm_provider": "",
                    "llm_model": "",
                    "builtin": true,
                    "prompt_sealed": true,
                })];
                agents.extend(
                    reg.names()
                        .into_iter()
                        .filter(|n| !n.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID))
                        .filter_map(|n| {
                            reg.get(&n).map(|d| {
                                serde_json::json!({
                                    "name": d.name,
                                    "kind": d.kind,
                                    "llm": d.llm,
                                    "llm_provider": d.llm_provider,
                                    "llm_model": d.llm_model,
                                    "builtin": false,
                                    "prompt_sealed": false,
                                })
                            })
                        }),
                );
                ControlResponse::Data {
                    payload: serde_json::json!({ "agents": agents }),
                }
            }

            ControlRequest::AgentRegistryLoad {
                workspace_root,
                name,
            } => {
                // Intercept the crony builtin: return its sealed definition.
                if name.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
                    let def = crate::crony::CronyBuiltin::def();
                    return ControlResponse::Data {
                        payload: serde_json::json!({
                            "name": def.name,
                            "kind": def.kind.as_str(),
                            "llm": "",
                            "llm_provider": "",
                            "llm_model": "",
                            "system_prompt": def.system_prompt,
                            "memory_namespace": def.memory_namespace,
                            "tools": def.tools,
                            "builtin": true,
                            "prompt_sealed": true,
                        }),
                    };
                }
                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                match reg.get(&name) {
                    Some(d) => ControlResponse::Data {
                        payload: serde_json::json!({
                            "name": d.name,
                            "kind": d.kind,
                            "llm": d.llm,
                            "llm_provider": d.llm_provider,
                            "llm_model": d.llm_model,
                            "system_prompt": d.system_prompt,
                            "memory_namespace": d.memory_namespace,
                            "tools": d.tools,
                            "reasoning_effort": d.reasoning_effort,
                            "builtin": false,
                            "prompt_sealed": false,
                        }),
                    },
                    None => ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("agent not found: {name}"),
                        },
                    },
                }
            }

            ControlRequest::AgentRegistrySave {
                workspace_root,
                name,
                agent_kind,
                llm,
                system_prompt,
                memory_namespace,
                tools_csv,
            } => {
                // Guard: the Crony builtin prompt is sealed.
                if name.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: "crony is a builtin agent: its system prompt cannot be overwritten. \
                                     Create crony.agent.yaml to override peripheral fields only (reflection, vars, memory_namespace).".into(),
                        },
                    };
                }

                // Build canonical YAML from the structured fields.
                let effective_kind = if agent_kind == "reviewer" {
                    "reviewer"
                } else {
                    "worker"
                };
                let tools: Vec<&str> = tools_csv
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                let tools_yaml = if tools.is_empty() {
                    "[]".to_string()
                } else {
                    let items = tools
                        .iter()
                        .map(|t| format!("  - {t}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("\n{items}")
                };
                let effective_mem = if memory_namespace.is_empty() {
                    name.clone()
                } else {
                    memory_namespace.clone()
                };
                let yaml = format!(
                    "name: {name}\nkind: {effective_kind}\nllm: {llm}\nsystem_prompt: |\n  {sp}\nmemory_namespace: {effective_mem}\ntools:{tools_yaml}\n",
                    sp = system_prompt.replace('\n', "\n  "),
                );

                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                match reg.save(&name, &yaml).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            ControlRequest::AgentRegistryDelete {
                workspace_root,
                name,
            } => {
                // Guard: the Crony builtin cannot be deleted.
                if name.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: "crony is a builtin agent and cannot be deleted.".into(),
                        },
                    };
                }
                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                match reg.delete(&name).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            // ── Phase 3: Doc-type registry ────────────────────────────────
            ControlRequest::DocTypeList {
                workspace_root,
                builtin_doc_types_dir,
            } => {
                use crate::workspace::{DocTypeRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let builtin = builtin_doc_types_dir
                    .as_deref()
                    .map(std::path::Path::new)
                    .unwrap_or(std::path::Path::new(""))
                    .to_owned();
                let mut reg = DocTypeRegistry::new(builtin, layout.doc_types_dir());
                reg.refresh().await;
                let types: Vec<serde_json::Value> = reg
                    .names()
                    .into_iter()
                    .filter_map(|n| {
                        reg.get(&n).map(|s| {
                            serde_json::json!({
                                "name": s.name,
                                "display_name": s.display_name,
                                "user_defined": s.user_defined,
                            })
                        })
                    })
                    .collect();
                ControlResponse::Data {
                    payload: serde_json::json!({ "doc_types": types }),
                }
            }

            ControlRequest::DocTypeLoad {
                workspace_root,
                builtin_doc_types_dir,
                name,
            } => {
                use crate::workspace::{DocTypeRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let builtin = builtin_doc_types_dir
                    .as_deref()
                    .map(std::path::Path::new)
                    .unwrap_or(std::path::Path::new(""))
                    .to_owned();
                let mut reg = DocTypeRegistry::new(builtin, layout.doc_types_dir());
                reg.refresh().await;
                match reg.get(&name) {
                    Some(s) => ControlResponse::Data {
                        payload: serde_json::json!({
                            "name": s.name,
                            "display_name": s.display_name,
                            "description": s.description,
                            "user_defined": s.user_defined,
                        }),
                    },
                    None => ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("doc type not found: {name}"),
                        },
                    },
                }
            }

            ControlRequest::DocTypeSave {
                workspace_root,
                name,
                display_name,
                description,
            } => {
                use crate::workspace::{DocTypeRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = DocTypeRegistry::new("", layout.doc_types_dir());
                match reg.save(&name, &display_name, &description).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            ControlRequest::DocTypeDelete {
                workspace_root,
                name,
            } => {
                use crate::workspace::{DocTypeRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = DocTypeRegistry::new("", layout.doc_types_dir());
                match reg.delete(&name).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            // ── Phase 4: Terminal PTY sessions ────────────────────────────
            ControlRequest::TerminalStart {
                terminal_id,
                workspace_root,
                shell,
                cols,
                rows,
            } => {
                let cols = cols.unwrap_or(100);
                let rows = rows.unwrap_or(30);
                let shell = shell.unwrap_or_else(|| "/bin/zsh".to_owned());
                let cwd = std::path::PathBuf::from(&workspace_root);
                let tid = terminal_id.clone();

                // Get (or create) the shared session manager for this workspace.
                let mgr = {
                    let mut map = self.services.terminal_managers.lock();
                    map.entry(workspace_root.clone())
                        .or_insert_with(crate::terminal::PtySessionManager::new_shared)
                        .clone()
                };

                // Capture the authority so the output closure can push events
                // on the subscription bus under topic "terminal:<id>".
                let authority = self.authority.clone();

                {
                    let tid_out = tid.clone();
                    let mut mgr_guard = mgr.lock().await;
                    if let Err(e) = mgr_guard
                        .create(
                            tid.clone(),
                            cwd,
                            &shell,
                            cols,
                            rows,
                            move |chunk| {
                                let data_b64 = base64_encode(&chunk);
                                authority.emit(
                                    format!("terminal:{tid_out}"),
                                    crate::protocol::events::RuntimeEventPayload::Raw {
                                        data: serde_json::json!({
                                            "id": tid_out,
                                            "data": data_b64,
                                        }),
                                    },
                                );
                            },
                            move |_code| { /* exit handled on renderer side */ },
                        )
                        .await
                    {
                        return ControlResponse::Err {
                            error: ControlError::Internal {
                                message: e.to_string(),
                            },
                        };
                    }
                };

                ControlResponse::Data {
                    payload: serde_json::json!({ "session_id": tid }),
                }
            }

            ControlRequest::TerminalInput { terminal_id, data } => {
                let mgr = {
                    let map = self.services.terminal_managers.lock();
                    // Find any manager that has this terminal_id (we stored by workspace_root,
                    // so iterate). In practice each terminal_id is globally unique.
                    map.values().next().cloned()
                };
                if let Some(mgr) = mgr {
                    let guard = mgr.lock().await;
                    let _ = guard.write(&terminal_id, data.as_bytes());
                }
                ControlResponse::Ack
            }

            ControlRequest::TerminalResize {
                terminal_id,
                cols,
                rows,
            } => {
                let mgr = {
                    let map = self.services.terminal_managers.lock();
                    map.values().next().cloned()
                };
                if let Some(mgr) = mgr {
                    let guard = mgr.lock().await;
                    let _ = guard.resize(&terminal_id, cols, rows);
                }
                ControlResponse::Ack
            }

            ControlRequest::TerminalStop { terminal_id } => {
                let mgr = {
                    let map = self.services.terminal_managers.lock();
                    map.values().next().cloned()
                };
                if let Some(mgr) = mgr {
                    let mut guard = mgr.lock().await;
                    guard.close(&terminal_id);
                }
                ControlResponse::Ack
            }

            // ── Document store ────────────────────────────────────────────
            ControlRequest::DocumentList {
                workspace_root,
                flow_id,
            } => {
                let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    crate::flow::document::DocumentStore::new(flow_dir).list()
                })
                .await
                .unwrap_or_default();

                let docs: Vec<serde_json::Value> = result
                    .into_iter()
                    .map(|d| {
                        serde_json::json!({
                            "name": d.name,
                            "latest_revision": d.latest_revision,
                            "size_bytes": d.size_bytes,
                        })
                    })
                    .collect();
                ControlResponse::Data {
                    payload: serde_json::json!({ "docs": docs }),
                }
            }

            ControlRequest::DocumentRead {
                workspace_root,
                flow_id,
                name,
                revision,
            } => {
                let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    let store = crate::flow::document::DocumentStore::new(flow_dir);
                    if let Some(rev) = revision {
                        let content = store.read_revision(&name, rev)?;
                        Ok::<_, anyhow::Error>((rev, content))
                    } else {
                        let rev = store.latest_revision(&name);
                        let content = store.read(&name)?;
                        Ok((rev, content))
                    }
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking: {}", e)));

                match result {
                    Ok((rev, Some(content))) => ControlResponse::Data {
                        payload: serde_json::json!({ "revision": rev, "content": content }),
                    },
                    Ok((_, None)) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: "document not found".into(),
                        },
                    },
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            ControlRequest::DocumentSubmit {
                workspace_root,
                flow_id,
                name,
                content,
            } => {
                let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    crate::flow::document::DocumentStore::new(flow_dir)
                        .submit(&name, &content, 5000)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking: {}", e)));

                match result {
                    Ok(wr) => ControlResponse::Data {
                        payload: serde_json::json!({
                            "revision": wr.revision,
                            "sha256": wr.sha256_hex,
                        }),
                    },
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            ControlRequest::DocumentSuggestionApply {
                workspace_root,
                flow_id,
                run_id: _,
                name,
                block_id,
                suggestion,
            } => {
                let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    crate::flow::document::DocumentStore::new(flow_dir).suggestion_apply(
                        &name,
                        &block_id,
                        &suggestion,
                        5000,
                    )
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking: {}", e)));

                match result {
                    Ok(wr) => ControlResponse::Data {
                        payload: serde_json::json!({
                            "new_revision": wr.revision,
                            "sha": wr.sha256_hex,
                        }),
                    },
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }

            ControlRequest::MentionParse {
                workspace_root,
                flow_id,
                text,
            } => {
                use crate::workspace::{load_flow_agents, Workspace};
                let path = Workspace::new(&workspace_root).flow_file(&flow_id);
                let known: std::collections::HashSet<String> =
                    load_flow_agents(&path).await.into_iter().collect();

                // @mention parser: @[a-zA-Z_][a-zA-Z0-9_-]*
                let chars: Vec<char> = text.chars().collect();
                let mut mentions: Vec<serde_json::Value> = Vec::new();
                let mut unknown: Vec<serde_json::Value> = Vec::new();
                let mut i = 0usize;
                while i < chars.len() {
                    if chars[i] != '@' {
                        i += 1;
                        continue;
                    }
                    // Skip if preceded by alphanumeric / '_'
                    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
                        i += 1;
                        continue;
                    }
                    let mut j = i + 1;
                    if j >= chars.len() || (!chars[j].is_alphabetic() && chars[j] != '_') {
                        i += 1;
                        continue;
                    }
                    while j < chars.len()
                        && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-')
                    {
                        j += 1;
                    }
                    let name: String = chars[i + 1..j].iter().collect();
                    if known.contains(&name) {
                        mentions.push(serde_json::Value::String(name));
                    } else {
                        unknown.push(serde_json::Value::String(name));
                    }
                    i = j;
                }

                ControlResponse::Data {
                    payload: serde_json::json!({ "mentions": mentions, "unknown": unknown }),
                }
            }

            ControlRequest::GetSpaceSnapshot { space_id } => {
                let (runs, reviews) = self.authority.get_space_snapshot(&space_id);
                let runs_json: Vec<serde_json::Value> = runs
                    .into_iter()
                    .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
                    .collect();
                let reviews_json: Vec<serde_json::Value> = reviews
                    .into_iter()
                    .map(|rv| serde_json::to_value(rv).unwrap_or(serde_json::Value::Null))
                    .collect();
                ControlResponse::SpaceSnapshot {
                    runs: runs_json,
                    pending_reviews: reviews_json,
                }
            }

            ControlRequest::SessionList { workspace_root } => {
                // Derive the workspace_cache_dir from workspace_root using the
                // same convention as StoragePaths: <workspace_root>/.cronymax/
                let cache_dir = std::path::PathBuf::from(&workspace_root).join(".cronymax");
                let store = crate::runtime::chat_store::ChatStore::new(&cache_dir);
                let sessions = store.list_sessions();
                let sessions_json: Vec<serde_json::Value> = sessions
                    .into_iter()
                    .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
                    .collect();
                ControlResponse::Data {
                    payload: serde_json::json!({ "sessions": sessions_json }),
                }
            }

            ControlRequest::SessionThreadInspect {
                workspace_root,
                session_id,
            } => {
                let cache_dir = std::path::PathBuf::from(&workspace_root).join(".cronymax");
                let store = crate::runtime::chat_store::ChatStore::new(&cache_dir);
                let sid = SessionId::from(session_id.as_str());

                // Try live authority first (session may be in memory), then
                // fall back to disk via ChatStore.
                let messages = self
                    .authority
                    .session_thread(&sid)
                    .unwrap_or_else(|| store.load_history(&sid));

                let turn_count = messages.len();
                let messages_json: Vec<serde_json::Value> = messages
                    .into_iter()
                    .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
                    .collect();
                ControlResponse::Data {
                    payload: serde_json::json!({
                        "messages": messages_json,
                        "turn_count": turn_count,
                    }),
                }
            }
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
        let id = match parse_run(run_id_str) {
            Ok(r) => r,
            Err(resp) => return resp,
        };
        match op(&self.authority, id) {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: authority_err_to_control(e, None, Some(run_id_str)),
            },
        }
    }
}

/// Base64-encode bytes for terminal output events.
/// Minimal default system prompt for ad-hoc chat (no flow, no authored
/// agent yaml). Without this, agentic models like gpt-4o keep calling
/// tools until max_turns runs out because nothing tells them when the
/// task is "done enough" to write a final answer.
fn default_chat_system_prompt() -> String {
    [
        "You are a helpful coding assistant operating inside a developer's workspace.",
        "You may use the provided tools (shell, filesystem, etc.) to investigate the project, but only when the user's request actually requires it.",
        "Use the minimum number of tool calls needed. As soon as you have enough information to answer, stop calling tools and reply to the user with a concise summary.",
        "If the request is purely conversational, answer directly without invoking any tool.",
    ]
    .join(" ")
}

fn base64_encode(data: &[u8]) -> String {
    // Simple base64 without dependencies — use the alphabet directly.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32;
        out.push(ALPHABET[(n >> 18) as usize]);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize]);
        out.push(ALPHABET[(n & 0x3f) as usize]);
        i += 3;
    }
    match data.len() - i {
        1 => {
            let n = (data[i] as u32) << 16;
            out.push(ALPHABET[(n >> 18) as usize]);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
            out.extend_from_slice(b"==");
        }
        2 => {
            let n = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8;
            out.push(ALPHABET[(n >> 18) as usize]);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize]);
            out.push(b'=');
        }
        _ => {}
    }
    String::from_utf8(out).unwrap_or_default()
}

fn parse_run(s: &str) -> Result<RunId, ControlResponse> {
    Uuid::parse_str(s)
        .map(RunId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid run id: {s}"),
            },
        })
}

fn parse_space(s: &str) -> Result<SpaceId, ControlResponse> {
    Uuid::parse_str(s)
        .map(SpaceId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid space id: {s}"),
            },
        })
}

fn parse_review(s: &str) -> Result<ReviewId, ControlResponse> {
    Uuid::parse_str(s)
        .map(ReviewId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid review id: {s}"),
            },
        })
}

fn authority_err_to_control(
    e: AuthorityError,
    space_id: Option<&str>,
    run_id: Option<&str>,
) -> ControlError {
    match e {
        AuthorityError::UnknownSpace(id) => ControlError::UnknownSpace {
            space_id: space_id
                .map(str::to_owned)
                .unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownRun(id) => ControlError::UnknownRun {
            run_id: run_id.map(str::to_owned).unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownReview(_) => ControlError::InvalidRequest {
            message: "unknown review".into(),
        },
        AuthorityError::InvalidTransition { state, action, .. } => ControlError::InvalidState {
            message: format!("cannot {action} from {state:?}"),
        },
        AuthorityError::ReviewAlreadyResolved => ControlError::InvalidState {
            message: "review already resolved".into(),
        },
        AuthorityError::Persistence(p) => ControlError::Internal {
            message: format!("persistence: {p}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::protocol::dispatch::run as dispatch_run;
    use crate::protocol::envelope::ClientToRuntime;
    use crate::protocol::transport::memory;
    use crate::protocol::version::PROTOCOL_VERSION;
    use crate::runtime::state::Space;

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
            Arc::new(RuntimeServices::new_minimal(
                auth.clone(),
                Arc::new(Mutex::new(HashMap::new())),
            )),
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
        let subscription = match client.recv().await.unwrap() {
            RuntimeToClient::Control {
                response: ControlResponse::Subscribed { subscription },
                ..
            } => subscription,
            other => panic!("expected Subscribed, got {other:?}"),
        };

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
                },
            })
            .await
            .unwrap();
        let _run_id = match client.recv().await.unwrap() {
            RuntimeToClient::Control {
                response: ControlResponse::RunStarted { run_id, .. },
                ..
            } => run_id,
            other => panic!("expected RunStarted, got {other:?}"),
        };

        // Expect the resulting Event message on the subscription.
        match client.recv().await.unwrap() {
            RuntimeToClient::Event {
                subscription: s,
                event,
            } => {
                assert_eq!(s, subscription);
                assert_eq!(event.sequence, 0);
            }
            other => panic!("expected Event, got {other:?}"),
        }

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
            Arc::new(RuntimeServices::new_minimal(
                auth.clone(),
                Arc::new(Mutex::new(HashMap::new())),
            )),
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
}
