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

use crate::agent_loop::{LoopConfig, ReactLoop};
use crate::capability::agent_loader;
use crate::capability::dispatcher::HostCapabilityDispatcher;
use crate::capability::filesystem::{LocalFilesystem, WorkspaceScope};
use crate::capability::notify::NullNotify;
use crate::capability::shell::LocalShell;
use crate::capability::submit_document::DocumentSubmitted;
use crate::flow::definition::FlowDefinition;
use crate::flow::runtime::{FlowRuntime, InvocationContext, InvocationTrigger};
use crate::llm::{copilot_auth, OpenAiConfig, OpenAiProvider};
use crate::protocol::capabilities::CapabilityResponse;
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse, ReviewDecision};
use crate::protocol::dispatch::{Handler, ResponseSink};
use crate::protocol::envelope::{CorrelationId, RuntimeToClient, SubscriptionId};
use crate::sandbox::broker::PermissionBroker;
use crate::sandbox::policy::SandboxPolicy;
use crate::sandbox::fs_gate::PolicyFilesystem;
use crate::sandbox::shell_gate::PolicyShell;
use uuid::Uuid;

use super::authority::{AuthorityError, RuntimeAuthority, SubscribeOutcome};
use super::state::{PermissionState, RunId, ReviewId, SpaceId};

// ── Shared context for one flow run ──────────────────────────────────────────

/// Context cloned into the supervision task and agent-spawn helper so
/// all downstream agent invocations share the same LLM and workspace config.
#[derive(Clone)]
struct FlowRunContext {
    authority: RuntimeAuthority,
    space_id: SpaceId,
    workspace_root: PathBuf,
    flow_id: String,
    flow_run_id: String,
    flow_runtime: Arc<FlowRuntime>,
    doc_tx: tokio::sync::mpsc::Sender<DocumentSubmitted>,
    base_url: String,
    api_key: Option<String>,
    model: String,
    provider_kind: String,
    /// Sandbox policy for capability gates. `None` = permissive (no checks).
    sandbox_policy: Option<Arc<SandboxPolicy>>,
}

impl std::fmt::Debug for FlowRunContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowRunContext")
            .field("flow_id", &self.flow_id)
            .field("flow_run_id", &self.flow_run_id)
            .finish()
    }
}

/// Spawn a `ReactLoop` for one agent invocation within a flow run.
/// Creates its own `authority_run_id` so the authority's lifecycle
/// tracking does not interfere with the parent flow run.
///
/// The agent definition is loaded from
/// `<workspace>/.cronymax/agents/<agent_id>.agent.yaml` (if present) so
/// the LLM receives the agent's authored `system_prompt`, the appropriate
/// model override, and a correctly-scoped tool allow-list.
fn spawn_agent_loop(ctx: FlowRunContext, agent_id: String, inv_ctx: InvocationContext) {
    let run_id = match ctx.authority.start_run(ctx.space_id, None, serde_json::json!({})) {
        Ok(id) => id,
        Err(e) => {
            warn!(agent_id, error = %e, "spawn_agent_loop: authority.start_run failed");
            return;
        }
    };

    tokio::spawn(async move {
        // ── Load agent definition ─────────────────────────────────────────
        let agent_def = agent_loader::load_agent(&ctx.workspace_root, &agent_id).await;

        // Build the effective system prompt: agent's persona first, then the
        // flow-context message (task info, port state, etc.).
        let system_message = if agent_def.system_prompt.is_empty() {
            inv_ctx.system_message.clone()
        } else {
            format!(
                "{}\n\n---\n\n{}",
                agent_def.system_prompt, inv_ctx.system_message
            )
        };

        // Use agent's declared model when available, falling back to the
        // flow-run default model.
        let model = if agent_def.llm_model.is_empty() {
            ctx.model.clone()
        } else {
            agent_def.llm_model.clone()
        };

        // Determine runner kind from agent's `kind` field.
        let runner_role = agent_def.kind.as_str();

        // ── Build capability dispatcher ───────────────────────────────────
        let broker = PermissionBroker::new();
        let mut cap_builder = HostCapabilityDispatcher::builder();

        // Shell capability — check SandboxPolicy before execution.
        if let Some(policy) = &ctx.sandbox_policy {
            let shell_cap = PolicyShell::new(
                LocalShell::new(&ctx.workspace_root),
                broker.clone(),
                Arc::clone(policy),
            );
            cap_builder.register_shell(Arc::new(shell_cap), true);
        } else {
            cap_builder.register_shell(Arc::new(LocalShell::new(&ctx.workspace_root)), true);
        }

        // Filesystem capability — enforce WorkspaceScope + optional SandboxPolicy.
        let scope = WorkspaceScope::new(&ctx.workspace_root);
        if let Some(policy) = &ctx.sandbox_policy {
            let fs_cap = PolicyFilesystem::new(LocalFilesystem, broker, Arc::clone(policy));
            cap_builder.register_filesystem(Arc::new(fs_cap), scope);
        } else {
            cap_builder.register_filesystem(Arc::new(LocalFilesystem), scope);
        }
        cap_builder.register_notify(Arc::new(NullNotify));
        let store = crate::capability::test_runner::LastReportStore::new();
        cap_builder.register_test_runner(
            ctx.workspace_root.clone(),
            store,
            ctx.flow_run_id.clone(),
            runner_role,
        );
        cap_builder.register_submit_document(
            ctx.workspace_root.clone(),
            ctx.flow_id.clone(),
            ctx.flow_run_id.clone(),
            agent_id.clone(),
            ctx.doc_tx.clone(),
        );
        let tools = Arc::new(cap_builder.build());

        let authority = ctx.authority.clone();
        let base_url = ctx.base_url.clone();
        let raw_api_key = ctx.api_key.clone();
        let is_copilot = ctx.provider_kind == "github_copilot";

        // For GitHub Copilot, exchange the stored GitHub OAuth token for the
        // short-lived Copilot API token required by api.githubcopilot.com.
        let (api_key, copilot_mode) = if is_copilot {
            match raw_api_key.as_deref() {
                Some(github_token) if !github_token.is_empty() => {
                    let http = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(30))
                        .build()
                        .unwrap_or_default();
                    match copilot_auth::exchange_for_copilot_token(&http, github_token).await {
                        Ok(ct) => {
                            info!(agent_id, "spawn_agent_loop: copilot token exchanged successfully");
                            (Some(ct.token), true)
                        }
                        Err(e) => {
                            warn!(agent_id, error = %e, "spawn_agent_loop: copilot token exchange failed, attempting with raw token");
                            (raw_api_key, true)
                        }
                    }
                }
                _ => (raw_api_key, true),
            }
        } else {
            (raw_api_key, false)
        };

        let llm_cfg = OpenAiConfig {
            base_url: base_url.clone(),
            api_key,
            default_model: model.clone(),
            copilot_mode,
            ..Default::default()
        };
        let llm = match OpenAiProvider::new(llm_cfg) {
            Ok(p) => p,
            Err(e) => {
                warn!(agent_id, error = %e, "spawn_agent_loop: OpenAiProvider::new failed");
                let _ = authority.fail_run(run_id, e.to_string());
                return;
            }
        };
        let cfg = LoopConfig {
            model: model.clone(),
            system_prompt: Some(system_message.clone()),
            user_input: "Continue with your assigned task as described above.".to_owned(),
            max_turns: 20,
            temperature: None,
            llm: Arc::new(llm),
            tools,
        };
        let result = ReactLoop::new(authority.clone(), run_id, cfg).run().await;
        info!(agent_id, %run_id, ok = result.is_ok(), "spawn_agent_loop: agent loop finished");
        if let Err(e) = result {
            info!(agent_id, %run_id, error = %e, "spawn_agent_loop: agent loop failed");
        }
    });
}

/// Adapter that turns a [`RuntimeAuthority`] into a dispatch
/// [`Handler`].
#[derive(Debug)]
pub struct RuntimeHandler {
    authority: RuntimeAuthority,
    /// Workspace roots passed at construction time (from `StoragePaths`).
    workspace_roots: Vec<PathBuf>,
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
    flow_contexts: Mutex<HashMap<String /* flow_run_id */, FlowRunContext>>,
    /// One-shot senders awaiting a `CapabilityReply` from the C++ host.
    /// Keyed by the `CorrelationId` that was sent with the `CapabilityCall`.
    pending_capabilities:
        Mutex<HashMap<CorrelationId, tokio::sync::oneshot::Sender<CapabilityResponse>>>,
    /// Per-workspace PTY session managers (Phase 4).
    /// Keyed by workspace_root string so each workspace gets its own manager.
    /// Wrapped in Arc so the map can be shared across multiple RuntimeHandler
    /// instances that serve different transports (browser vs renderer).
    terminal_managers:
        Arc<Mutex<HashMap<String, crate::terminal::SharedSessionManager>>>,
}

impl RuntimeHandler {
    pub fn new(authority: RuntimeAuthority, workspace_roots: Vec<PathBuf>) -> Self {
        Self::with_policy(authority, workspace_roots, None)
    }

    /// Construct with an explicit sandbox policy (built from `RuntimeConfig.sandbox`).
    pub fn with_policy(
        authority: RuntimeAuthority,
        workspace_roots: Vec<PathBuf>,
        sandbox_policy: Option<SandboxPolicy>,
    ) -> Self {
        Self::with_policy_and_managers(authority, workspace_roots, sandbox_policy, None)
    }

    /// Construct with an explicit sandbox policy and an optional shared terminal
    /// managers map. Pass the same `Arc` to multiple handlers to let them all
    /// access the same PTY sessions regardless of which transport created them.
    pub fn with_policy_and_managers(
        authority: RuntimeAuthority,
        workspace_roots: Vec<PathBuf>,
        sandbox_policy: Option<SandboxPolicy>,
        terminal_managers: Option<Arc<Mutex<HashMap<String, crate::terminal::SharedSessionManager>>>>,
    ) -> Self {
        Self {
            authority,
            workspace_roots,
            sandbox_policy: sandbox_policy.map(Arc::new),
            sink: Mutex::new(None),
            fanout: Mutex::new(HashMap::new()),
            flow_contexts: Mutex::new(HashMap::new()),
            pending_capabilities: Mutex::new(HashMap::new()),
            terminal_managers: terminal_managers
                .unwrap_or_else(|| Arc::new(Mutex::new(HashMap::new()))),
        }
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

        let sink = self.sink.lock().clone().ok_or_else(|| {
            anyhow::anyhow!("call_capability: no active transport sink")
        })?;
        sink.send(RuntimeToClient::CapabilityCall { id, request })
            .await
            .map_err(|_| anyhow::anyhow!("call_capability: transport sink closed"))?;

        rx.await.map_err(|_| anyhow::anyhow!("call_capability: sender dropped (disconnected?)")
        )
    }
}

#[async_trait]
impl Handler for RuntimeHandler {
    async fn on_connected(&self, sink: ResponseSink) {
        *self.sink.lock() = Some(sink);
    }

    async fn handle_control(
        &self,
        _id: CorrelationId,
        request: ControlRequest,
    ) -> ControlResponse {
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
                let SubscribeOutcome { id, mut receiver } =
                    self.authority.subscribe(topic);
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

            ControlRequest::StartRun { space_id, payload } => {
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
                    .unwrap_or_else(|| std::env::temp_dir());

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
                match self.authority.start_run(space, None, payload) {
                    Ok(run_id) => {
                        info!(%run_id, "start_run: created run, setting up fan-out");
                        let sub_outcome = self
                            .authority
                            .subscribe(format!("run:{run_id}"));
                        let sub_id = sub_outcome.id;
                        let mut receiver = sub_outcome.receiver;
                        if let Some(sink) = self.sink.lock().clone() {
                            let task = tokio::spawn(async move {
                                while let Some(event) = receiver.recv().await {
                                    let kind = match &event.payload {
                                        crate::protocol::events::RuntimeEventPayload::RunStatus { status, .. } => format!("run_status:{status}"),
                                        crate::protocol::events::RuntimeEventPayload::Token { .. } => "token".into(),
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

                        // Optionally create a FlowRuntime + initial context
                        // when the request carries a `flow_id`.
                        let (entry_system_prompt, maybe_flow_ctx) = if let Some(ref fid) = flow_id_opt {
                            let flow_rt = Arc::new(FlowRuntime::new(&workspace_root));
                            let flow_run_id = match flow_rt.start_run(fid, &initial_input).await {
                                Ok(id) => id,
                                Err(e) => {
                                    warn!(flow_id = %fid, error = %e, "start_run: FlowRuntime::start_run failed");
                                    let _ = self.authority.fail_run(run_id, e.to_string());
                                    return ControlResponse::Err {
                                        error: ControlError::Internal { message: e.to_string() },
                                    };
                                }
                            };
                            info!(%run_id, %flow_run_id, flow_id = %fid, "start_run: flow run created");

                            // Load the flow definition to schedule the entry agent.
                            let flow_def_path = workspace_root
                                .join(".cronymax")
                                .join("flows")
                                .join(fid)
                                .join("flow.yaml");

                            let entry_sys = match tokio::fs::read_to_string(&flow_def_path).await {
                                Ok(yaml) => {
                                    match FlowDefinition::load_from_str(&yaml, &flow_def_path) {
                                        Ok(flow_def) => {
                                            // Schedule the entry agent (first agent in the flow).
                                            if let Some(entry_agent) = flow_def.agents.iter().next().cloned() {
                                                let trigger = InvocationTrigger {
                                                    kind: "initial".into(),
                                                    approved_port: None,
                                                    approved_doc: None,
                                                };
                                                match flow_rt.schedule_agent_with_context(
                                                    &flow_run_id, &entry_agent, trigger, &flow_def,
                                                ).await {
                                                    Ok(Some(ctx)) => {
                                                        info!(entry_agent, "start_run: entry agent scheduled");
                                                        Some(ctx.system_message)
                                                    }
                                                    _ => None,
                                                }
                                            } else {
                                                warn!(flow_id = %fid, "start_run: flow has no agents");
                                                None
                                            }
                                        }
                                        Err(e) => {
                                            warn!(flow_id = %fid, error = %e, "start_run: failed to parse flow.yaml");
                                            None
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(flow_id = %fid, error = %e, "start_run: failed to read flow.yaml");
                                    None
                                }
                            };

                            let flow_ctx = FlowRunContext {
                                authority: self.authority.clone(),
                                space_id: space,
                                workspace_root: workspace_root.clone(),
                                flow_id: fid.clone(),
                                flow_run_id: flow_run_id.clone(),
                                flow_runtime: flow_rt.clone(),
                                doc_tx: doc_tx.clone(),
                                base_url: base_url.clone(),
                                api_key: api_key.clone(),
                                model: model.clone(),
                                provider_kind: provider_kind.clone(),
                                sandbox_policy: self.sandbox_policy.clone(),
                            };
                            self.flow_contexts.lock().insert(flow_run_id, flow_ctx.clone());
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
                            tokio::spawn(async move {
                                while let Some(evt) = doc_rx.recv().await {
                                    info!(
                                        run_id = %evt.run_id,
                                        agent_id = %evt.agent_id,
                                        doc_type = %evt.doc_type,
                                        document_id = %evt.document_id,
                                        "supervision: document submitted"
                                    );

                                    // Load flow definition fresh for routing.
                                    let flow_def_path = fctx.workspace_root
                                        .join(".cronymax")
                                        .join("flows")
                                        .join(&fctx.flow_id)
                                        .join("flow.yaml");

                                    let yaml = match tokio::fs::read_to_string(&flow_def_path).await {
                                        Ok(y) => y,
                                        Err(e) => {
                                            warn!(error = %e, "supervision: failed to read flow.yaml");
                                            continue;
                                        }
                                    };
                                    let flow_def = match FlowDefinition::load_from_str(&yaml, &flow_def_path) {
                                        Ok(d) => d,
                                        Err(e) => {
                                            warn!(error = %e, "supervision: failed to parse flow.yaml");
                                            continue;
                                        }
                                    };

                                    // Process the document submission.
                                    match fctx.flow_runtime.on_document_submitted(
                                        &evt.run_id,
                                        &evt.agent_id,
                                        &evt.doc_type,
                                        &evt.body,
                                        &flow_def,
                                        &evt.sha256,
                                        evt.revision,
                                    ).await {
                                        Ok(targets) => {
                                            for (agent_id, ctx_opt) in targets {
                                                if let Some(inv_ctx) = ctx_opt {
                                                    info!(
                                                        agent_id,
                                                        "supervision: spawning downstream agent"
                                                    );
                                                    spawn_agent_loop(fctx.clone(), agent_id, inv_ctx);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!(error = %e, run_id = %evt.run_id, "supervision: on_document_submitted failed");
                                            // Cycle limit exceeded or other terminal error — fail the run.
                                            if e.to_string().contains("cycle limit exceeded") {
                                                let _ = fctx.authority.fail_run(
                                                    RunId(Uuid::parse_str(&evt.run_id).unwrap_or_default()),
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
                        cap_builder.register_shell(Arc::new(LocalShell::new(&workspace_root)), true);
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
                                .map(|c| c.flow_run_id.clone())
                                .unwrap_or_else(|| run_id.to_string());
                            cap_builder.register_submit_document(
                                workspace_root.clone(),
                                fid.clone(),
                                flow_run_id_for_tool,
                                entry_agent_id,
                                doc_tx.clone(),
                            );
                        }
                        let tools = Arc::new(cap_builder.build());

                        let authority = self.authority.clone();
                        let is_copilot = provider_kind == "github_copilot";
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
                                        match copilot_auth::exchange_for_copilot_token(&http, github_token).await {
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

                            let llm_cfg = OpenAiConfig {
                                base_url: base_url.clone(),
                                api_key: effective_api_key,
                                default_model: model.clone(),
                                copilot_mode,
                                ..Default::default()
                            };
                            info!(%run_id, llm_base_url = %base_url, %model, "react_loop: starting");
                            let llm = match OpenAiProvider::new(llm_cfg) {
                                Ok(p) => p,
                                Err(e) => {
                                    info!(%run_id, error = %e, "react_loop: OpenAiProvider::new failed");
                                    let _ = authority.fail_run(
                                        run_id,
                                        e.to_string(),
                                    );
                                    return;
                                }
                            };
                            let cfg = LoopConfig {
                                model,
                                system_prompt: effective_system_prompt,
                                user_input,
                                max_turns: 20,
                                temperature: None,
                                llm: Arc::new(llm),
                                tools,
                            };
                            let result = ReactLoop::new(authority.clone(), run_id, cfg)
                                .run()
                                .await;
                            info!(%run_id, ok = result.is_ok(), "react_loop: finished");
                            if let Err(e) = result {
                                info!(%run_id, error = %e, "react_loop: failed with error");
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

            ControlRequest::CancelRun { run_id } => {
                self.run_op(&run_id, |a, id| a.cancel_run(id))
            }
            ControlRequest::PauseRun { run_id } => {
                self.run_op(&run_id, |a, id| a.pause_run(id))
            }
            ControlRequest::ResumeRun { run_id } => {
                self.run_op(&run_id, |a, id| a.resume_run(id))
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
                let flow_ctx_opt: Option<FlowRunContext> = {
                    let map = self.flow_contexts.lock();
                    // Try a direct lookup by run_id string.
                    map.values()
                        .find(|c| c.flow_run_id == run_id || c.flow_run_id == run.to_string())
                        .cloned()
                };

                if let Some(fctx) = flow_ctx_opt {
                    // Load flow definition for routing decisions.
                    let flow_def_path = fctx.workspace_root
                        .join(".cronymax")
                        .join("flows")
                        .join(&fctx.flow_id)
                        .join("flow.yaml");

                    // Spawn async work for the FlowRuntime call since
                    // handle_control is async but we don't want to block.
                    let flow_run_id = fctx.flow_run_id.clone();
                    let is_approve = perm_decision == PermissionState::Approved;
                    tokio::spawn(async move {
                        let yaml = match tokio::fs::read_to_string(&flow_def_path).await {
                            Ok(y) => y,
                            Err(e) => {
                                warn!(error = %e, "resolve_review: failed to read flow.yaml");
                                return;
                            }
                        };
                        let flow_def = match FlowDefinition::load_from_str(&yaml, &flow_def_path) {
                            Ok(d) => d,
                            Err(e) => {
                                warn!(error = %e, "resolve_review: failed to parse flow.yaml");
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
                            match fctx.flow_runtime.on_document_approved(
                                &flow_run_id,
                                producing_agent,
                                port,
                                &flow_def,
                            ).await {
                                Ok(Some(inv_ctx)) => {
                                    info!(producing_agent, port, "resolve_review: rescheduling after approval");
                                    spawn_agent_loop(fctx, producing_agent.to_owned(), inv_ctx);
                                }
                                Ok(None) => {
                                    info!(producing_agent, port, "resolve_review: approval, no reschedule needed");
                                }
                                Err(e) => {
                                    warn!(error = %e, "resolve_review: on_document_approved failed");
                                }
                            }
                        } else {
                            match fctx.flow_runtime.on_rejected_requeue(
                                &flow_run_id,
                                producing_agent,
                                port,
                                &flow_def,
                            ).await {
                                Ok(Some(inv_ctx)) => {
                                    info!(producing_agent, port, "resolve_review: requeueing after rejection");
                                    spawn_agent_loop(fctx, producing_agent.to_owned(), inv_ctx);
                                }
                                Ok(None) => {
                                    info!(producing_agent, port, "resolve_review: rejection, no requeue needed");
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
                match self.authority.resolve_review(run, review, perm_decision, notes) {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: authority_err_to_control(e, None, Some(&run_id)),
                    },
                }
            }

            // ── Phase 2: Workspace / file / flow handlers ─────────────────

            ControlRequest::WorkspaceLayout { workspace_root } => {
                use crate::workspace::WorkspaceLayout;
                let layout = WorkspaceLayout::new(&workspace_root);
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
                        "layout_version": crate::workspace::layout::LAYOUT_VERSION,
                    }),
                }
            }

            ControlRequest::FileRead { workspace_root, path } => {
                use crate::workspace::FileBroker;
                let broker = FileBroker::new(&workspace_root);
                match broker.read_text(std::path::Path::new(&path)).await {
                    Ok(content) => ControlResponse::Data {
                        payload: serde_json::json!({ "content": content }),
                    },
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }

            ControlRequest::FileWrite { workspace_root, path, content } => {
                use crate::workspace::FileBroker;
                let broker = FileBroker::new(&workspace_root);
                match broker.write_text(std::path::Path::new(&path), &content).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }

            ControlRequest::FlowList { workspace_root, builtin_flows_dir } => {
                use crate::workspace::{WorkspaceLayout, load_flow_yaml};
                let layout = WorkspaceLayout::new(&workspace_root);
                let mut flows: Vec<serde_json::Value> = Vec::new();
                let mut local_ids = std::collections::HashSet::new();

                // Scan workspace-local flows first.
                if let Ok(mut rd) = tokio::fs::read_dir(layout.flows_dir()).await {
                    while let Ok(Some(entry)) = rd.next_entry().await {
                        if !entry.path().is_dir() { continue; }
                        let id = entry.file_name().to_string_lossy().to_string();
                        let flow_yaml_path = entry.path().join("flow.yaml");
                        if let Some(doc) = load_flow_yaml(&flow_yaml_path, &id).await {
                            let agents: Vec<_> = doc.agents.iter().map(|a| &a.id).collect();
                            flows.push(serde_json::json!({
                                "id": id,
                                "name": doc.name,
                                "edge_count": doc.edges.len(),
                                "agents": agents,
                                "builtin": false,
                            }));
                            local_ids.insert(id);
                        }
                    }
                }

                // Merge builtin flows (dedup by id, workspace wins).
                if let Some(builtin_dir) = builtin_flows_dir {
                    if let Ok(mut rd) = tokio::fs::read_dir(&builtin_dir).await {
                        while let Ok(Some(entry)) = rd.next_entry().await {
                            if !entry.path().is_dir() { continue; }
                            let id = entry.file_name().to_string_lossy().to_string();
                            if local_ids.contains(&id) { continue; }
                            let flow_yaml_path = entry.path().join("flow.yaml");
                            if let Some(doc) = load_flow_yaml(&flow_yaml_path, &id).await {
                                let agents: Vec<_> = doc.agents.iter().map(|a| &a.id).collect();
                                flows.push(serde_json::json!({
                                    "id": id,
                                    "name": doc.name,
                                    "edge_count": doc.edges.len(),
                                    "agents": agents,
                                    "builtin": true,
                                }));
                            }
                        }
                    }
                }

                ControlResponse::Data { payload: serde_json::json!({ "flows": flows }) }
            }

            ControlRequest::FlowLoad { workspace_root, flow_id } => {
                use crate::workspace::{WorkspaceLayout, load_flow_yaml};
                let layout = WorkspaceLayout::new(&workspace_root);
                let path = layout.flow_file(&flow_id);
                match load_flow_yaml(&path, &flow_id).await {
                    Some(doc) => {
                        let agents: Vec<_> = doc.agents.iter().map(|a| &a.id).collect();
                        let edges: Vec<serde_json::Value> = doc.edges.iter().map(|e| {
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
                        }).collect();
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

            ControlRequest::FlowSave { workspace_root, flow_id, graph } => {
                use crate::workspace::{WorkspaceLayout, FlowYamlDoc, FlowYamlEdge};
                use crate::workspace::flow_yaml::flow_yaml_to_string;

                // Validate flow_id (alphanumeric + _ -)
                let valid = !flow_id.is_empty()
                    && flow_id.len() <= 64
                    && flow_id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
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
                        let agent_name = node.get("config")
                            .and_then(|c| c.get("agent_name"))
                            .and_then(|v| v.as_str())
                            .or_else(|| node.get("name").and_then(|v| v.as_str()))
                            .unwrap_or("")
                            .to_owned();
                        if agent_name.is_empty() || node_id < 0 { continue; }
                        agent_names.push(agent_name.clone());
                        id_to_agent.insert(node_id, agent_name);
                    }
                }

                let mut edges: Vec<FlowYamlEdge> = Vec::new();
                if let Some(edge_arr) = graph.get("edges").and_then(|v| v.as_array()) {
                    for e in edge_arr {
                        let from_id = e.get("from_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                        let to_id   = e.get("to_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                        let Some(from_agent) = id_to_agent.get(&from_id) else { continue };
                        let to_agent = id_to_agent.get(&to_id).cloned().unwrap_or_default();
                        edges.push(FlowYamlEdge {
                            from: from_agent.clone(),
                            to: to_agent,
                            port: e.get("port").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                            requires_human_approval: e.get("requires_human_approval")
                                .and_then(|v| v.as_bool()).unwrap_or(false),
                            ..Default::default()
                        });
                    }
                }

                let doc = FlowYamlDoc {
                    id: flow_id.clone(),
                    name: flow_id.clone(),
                    agents: agent_names.into_iter()
                        .map(|s| crate::workspace::flow_yaml::FlowYamlAgent { id: s })
                        .collect(),
                    edges,
                    ..Default::default()
                };

                let yaml = flow_yaml_to_string(&doc);
                let layout = WorkspaceLayout::new(&workspace_root);
                let flow_path = layout.flow_file(&flow_id);
                if let Some(parent) = flow_path.parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        return ControlResponse::Err {
                            error: ControlError::Internal { message: e.to_string() },
                        };
                    }
                }
                // Atomic write via temp file + rename.
                let tmp = flow_path.with_extension("yaml.tmp");
                if let Err(e) = tokio::fs::write(&tmp, &yaml).await {
                    return ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    };
                }
                if let Err(e) = tokio::fs::rename(&tmp, &flow_path).await {
                    return ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    };
                }
                ControlResponse::Ack
            }

            // ── Phase 3: Agent registry ───────────────────────────────────

            ControlRequest::AgentRegistryList { workspace_root } => {
                use crate::workspace::{AgentRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                let agents: Vec<serde_json::Value> = reg.names().into_iter()
                    .filter_map(|n| reg.get(&n).map(|d| serde_json::json!({
                        "name": d.name,
                        "kind": d.kind,
                        "llm": d.llm,
                        "llm_provider": d.llm_provider,
                        "llm_model": d.llm_model,
                    })))
                    .collect();
                ControlResponse::Data { payload: serde_json::json!({ "agents": agents }) }
            }

            ControlRequest::AgentRegistryLoad { workspace_root, name } => {
                use crate::workspace::{AgentRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
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
                        }),
                    },
                    None => ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("agent not found: {name}"),
                        },
                    },
                }
            }

            ControlRequest::AgentRegistrySave { workspace_root, name, yaml } => {
                use crate::workspace::{AgentRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                match reg.save(&name, &yaml).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }

            ControlRequest::AgentRegistryDelete { workspace_root, name } => {
                use crate::workspace::{AgentRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                match reg.delete(&name).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }

            // ── Phase 3: Doc-type registry ────────────────────────────────

            ControlRequest::DocTypeList { workspace_root, builtin_doc_types_dir } => {
                use crate::workspace::{DocTypeRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
                let builtin = builtin_doc_types_dir.as_deref()
                    .map(std::path::Path::new)
                    .unwrap_or(std::path::Path::new(""))
                    .to_owned();
                let mut reg = DocTypeRegistry::new(builtin, layout.doc_types_dir());
                reg.refresh().await;
                let types: Vec<serde_json::Value> = reg.names().into_iter()
                    .filter_map(|n| reg.get(&n).map(|s| serde_json::json!({
                        "name": s.name,
                        "display_name": s.display_name,
                        "user_defined": s.user_defined,
                    })))
                    .collect();
                ControlResponse::Data { payload: serde_json::json!({ "doc_types": types }) }
            }

            ControlRequest::DocTypeLoad { workspace_root, builtin_doc_types_dir, name } => {
                use crate::workspace::{DocTypeRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
                let builtin = builtin_doc_types_dir.as_deref()
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

            ControlRequest::DocTypeSave { workspace_root, name, display_name, description } => {
                use crate::workspace::{DocTypeRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
                let mut reg = DocTypeRegistry::new("", layout.doc_types_dir());
                match reg.save(&name, &display_name, &description).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }

            ControlRequest::DocTypeDelete { workspace_root, name } => {
                use crate::workspace::{DocTypeRegistry, WorkspaceLayout};
                let layout = WorkspaceLayout::new(&workspace_root);
                let mut reg = DocTypeRegistry::new("", layout.doc_types_dir());
                match reg.delete(&name).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }

            // ── Phase 4: Terminal PTY sessions ────────────────────────────

            ControlRequest::TerminalStart { terminal_id, workspace_root, shell, cols, rows } => {
                let cols = cols.unwrap_or(100);
                let rows = rows.unwrap_or(30);
                let shell = shell.unwrap_or_else(|| "/bin/zsh".to_owned());
                let cwd = std::path::PathBuf::from(&workspace_root);
                let tid = terminal_id.clone();

                // Get (or create) the shared session manager for this workspace.
                let mgr = {
                    let mut map = self.terminal_managers.lock();
                    map.entry(workspace_root.clone())
                        .or_insert_with(crate::terminal::new_shared)
                        .clone()
                };

                // Capture the authority so the output closure can push events
                // on the subscription bus under topic "terminal:<id>".
                let authority = self.authority.clone();

                {
                    let tid_out = tid.clone();
                    let mut mgr_guard = mgr.lock().await;
                    if let Err(e) = mgr_guard.create(
                        tid.clone(), cwd, &shell, cols, rows,
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
                    ).await {
                        return ControlResponse::Err {
                            error: ControlError::Internal { message: e.to_string() },
                        };
                    }
                };

                ControlResponse::Data {
                    payload: serde_json::json!({ "session_id": tid }),
                }
            }

            ControlRequest::TerminalInput { terminal_id, data } => {
                let mgr = {
                    let map = self.terminal_managers.lock();
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

            ControlRequest::TerminalResize { terminal_id, cols, rows } => {
                let mgr = {
                    let map = self.terminal_managers.lock();
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
                    let map = self.terminal_managers.lock();
                    map.values().next().cloned()
                };
                if let Some(mgr) = mgr {
                    let mut guard = mgr.lock().await;
                    guard.close(&terminal_id);
                }
                ControlResponse::Ack
            }

            // ── Document store ────────────────────────────────────────────

            ControlRequest::DocumentList { workspace_root, flow_id } => {
                let flow_dir =
                    crate::workspace::WorkspaceLayout::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    crate::document::DocumentStore::new(flow_dir).list()
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

            ControlRequest::DocumentRead { workspace_root, flow_id, name, revision } => {
                let flow_dir =
                    crate::workspace::WorkspaceLayout::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    let store = crate::document::DocumentStore::new(flow_dir);
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
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }

            ControlRequest::DocumentSubmit { workspace_root, flow_id, name, content } => {
                let flow_dir =
                    crate::workspace::WorkspaceLayout::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    crate::document::DocumentStore::new(flow_dir)
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
                        error: ControlError::Internal { message: e.to_string() },
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
                let flow_dir =
                    crate::workspace::WorkspaceLayout::new(&workspace_root).flow_dir(&flow_id);
                let result = tokio::task::spawn_blocking(move || {
                    crate::document::DocumentStore::new(flow_dir)
                        .suggestion_apply(&name, &block_id, &suggestion, 5000)
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
                        error: ControlError::Internal { message: e.to_string() },
                    },
                }
            }
        }
    }

    async fn handle_capability_reply(
        &self,
        id: CorrelationId,
        response: CapabilityResponse,
    ) {
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
fn base64_encode(data: &[u8]) -> String {
    // Simple base64 without dependencies — use the alphabet directly.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = (data[i] as u32) << 16 | (data[i+1] as u32) << 8 | data[i+2] as u32;
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
            let n = (data[i] as u32) << 16 | (data[i+1] as u32) << 8;
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
            space_id: space_id.map(str::to_owned).unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownRun(id) => ControlError::UnknownRun {
            run_id: run_id.map(str::to_owned).unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownReview(_) => ControlError::InvalidRequest {
            message: "unknown review".into(),
        },
        AuthorityError::InvalidTransition { state, action, .. } => {
            ControlError::InvalidState {
                message: format!("cannot {action} from {state:?}"),
            }
        }
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
        let space = Space { id: SpaceId::new(), name: "s".into() };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        let handler = Arc::new(RuntimeHandler::new(auth.clone(), vec![]));
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
            RuntimeToClient::Event { subscription: s, event } => {
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
        async fn handle_capability_reply(
            &self,
            id: CorrelationId,
            response: CapabilityResponse,
        ) {
            self.0.handle_capability_reply(id, response).await
        }
        async fn on_disconnected(&self) {
            self.0.on_disconnected().await
        }
    }
}
