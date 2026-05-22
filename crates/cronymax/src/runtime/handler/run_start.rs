//! StartRun handler — the main orchestration entry point.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::{info, warn};

use crate::agent_loop::tools::ToolDispatcher;
use crate::agent_loop::{
    maybe_compact, LoopConfig, ReactLoop, DEFAULT_RECENCY_TURNS, DEFAULT_THRESHOLD_PCT,
};
use crate::capability::agent_loader;
use crate::capability::dispatcher::HostCapabilityDispatcher;
use crate::capability::filesystem::{LocalFilesystem, WorkspaceScope};
use crate::capability::flow_tools::{register_flow_tools, SpawnAgentFn};
use crate::capability::invoke_agent::register_invoke_agent;
use crate::capability::invoke_flow::{build_invoke_flow_description, register_invoke_flow};
use crate::capability::notify::NullNotify;
use crate::capability::shell::LocalShell;
use crate::capability::submit_document::DocumentSubmitted;
use crate::capability::SandboxTier;
use crate::llm::LlmConfig;
use crate::llm::{
    copilot_auth, AnthropicConfig, AnthropicProvider, CapabilityResolver, OpenAiConfig,
    OpenAiProvider,
};
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};
use crate::protocol::envelope::RuntimeToClient;
use crate::runtime::run_context::RunContext;
use crate::runtime::state::{ForkPoint, RunId, SessionId, Space};
use uuid::Uuid;

use super::helpers::{
    apply_anthropic_effort_override, authority_err_to_control, build_middleware_chain,
    build_workspace_injection_block, parse_space,
};
use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_start_run(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::StartRun {
            space_id,
            payload,
            session_id,
            session_name,
            agent_id,
            child_session_id,
            goal: explicit_goal,
        } = req
        else {
            unreachable!()
        };
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
            .filter(|s| matches!(s.as_str(), "minimal" | "low" | "medium" | "high" | "xhigh"));
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

        // Pre-load agent definition for the chat agent (Crony builtin by default).
        // Used when no active flow context is set — i.e. either no flow_id in the
        // payload or the flow failed to start.  When a flow entry agent is active
        // (maybe_flow_ctx.is_some()) its own system-prompt and tools take over.
        // Done before `start_run_with_session` so no yield-points exist
        // between run creation (RunStatus:pending) and RunStarted reply.
        let resolved_agent_id = agent_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(crate::crony::CronyBuiltin::ID);
        let preloaded_chat_agent_def: Option<crate::capability::agent_loader::AgentDef> =
            Some(agent_loader::load_agent_with_builtin(&workspace_root, resolved_agent_id).await);

        // Resolve session: if session_id present, upsert the session
        // and retrieve the prior conversation thread from the ChatStore
        // (or fall back to the snapshot if ChatStore is not configured).
        let (maybe_session_id, prior_thread) = if let Some(ref sid) = session_id {
            let s_id = SessionId::from(sid.as_str());
            // Always upsert so run_ids tracking still works.
            let _ = self
                .authority
                .get_or_create_session(s_id.clone(), space, session_name.clone());
            // Load history from ChatStore if available, else fall back
            // to the snapshot thread (legacy / no workspace_cache_dir).
            // Crony (the supervisor) persists its conversation history across
            // turns even when a flow is selected, so the user's dialogue context
            // is preserved between messages.  Each spawned flow *agent* starts
            // fresh (handled in agent_runner::spawn_agent, not here).
            let thread = if let Some(ref cache_dir) = self.workspace_cache_dir {
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
        match self
            .authority
            .start_run_with_session(space, None, payload, maybe_session_id.clone())
        {
            Ok(run_id) => {
                // Resolve goal: explicit > flow_id string > first user message.
                // Done immediately after run creation so the label is visible
                // in the Activity Panel even before the run starts executing.
                let resolved_goal: Option<String> = explicit_goal
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| flow_id_opt.as_deref().map(str::to_string))
                    .or_else(|| {
                        let s = user_input.trim();
                        if s.is_empty() {
                            None
                        } else {
                            Some(s.chars().take(120).collect())
                        }
                    });
                if resolved_goal.is_some() {
                    let _ = self.authority.set_run_goal(run_id, resolved_goal);
                }
                info!(%run_id, "start_run: created run, setting up fan-out");
                let sub_outcome = self.authority.subscribe(format!("run:{run_id}"));
                let sub_id = sub_outcome.id;
                let mut receiver = sub_outcome.receiver;
                if let Some(sink) = self.sink.lock().clone() {
                    let task = tokio::spawn(async move {
                        while let Some(event) = receiver.recv().await {
                            let kind = match &event.payload {
                                crate::protocol::events::RuntimeEventPayload::RunStatus {
                                    status,
                                    ..
                                } => format!("run_status:{status}"),
                                crate::protocol::events::RuntimeEventPayload::Token { .. } => {
                                    "token".into()
                                }
                                crate::protocol::events::RuntimeEventPayload::ThinkingToken {
                                    ..
                                } => "thinking_token".into(),
                                crate::protocol::events::RuntimeEventPayload::Trace { .. } => {
                                    "trace".into()
                                }
                                crate::protocol::events::RuntimeEventPayload::Log { .. } => {
                                    "log".into()
                                }
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
                let (doc_tx, mut doc_rx) = tokio::sync::mpsc::channel::<DocumentSubmitted>(64);

                let ar = self.agent_runner.clone();

                // Optionally create a FlowRuntime + initial context
                // when the request carries a `flow_id`.
                let (entry_system_prompt, maybe_flow_ctx, entry_node_id) = if let Some(ref fid) =
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
                        .get_or_create(&workspace_root, self.workspace_cache_dir.as_deref())
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
                                        // Also record the flow_run_id in Session.flow_run_ids
                                        // so panels can discover all executions for this session.
                                        self.authority
                                            .attach_flow_run_to_session(sid, frid.clone());
                                    }
                                    // Bind the child session (or parent session) in the
                                    // authority's flow_sessions map so flow.run.changed and
                                    // other flow events are routed to session:{id} where the
                                    // frontend thread-view subscription can receive them.
                                    let bind_target = child_session_id
                                        .as_deref()
                                        .filter(|s| !s.is_empty())
                                        .map(str::to_owned)
                                        .or_else(|| maybe_session_id.as_ref().map(|s| s.0.clone()));
                                    if let Some(ref target) = bind_target {
                                        self.authority.bind_session(&frid, target);
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
                    // Also capture its node_id for submit_document routing before the
                    // vec is consumed by into_iter().
                    let entry_node_id = entry_contexts
                        .first()
                        .map(|c| c.node_id.clone())
                        .unwrap_or_default();

                    // If the caller supplied a child_session_id, upsert it and
                    // set its parent/fork_point so thread views can subscribe to it.
                    let maybe_child_session_id: Option<SessionId> = child_session_id
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            let child_sid = SessionId::from(s);
                            let _ = self.authority.get_or_create_session(
                                child_sid.clone(),
                                space,
                                None,
                            );
                            if let Some(ref parent_sid) = maybe_session_id {
                                self.authority.set_session_fork_point(
                                    &child_sid,
                                    parent_sid.clone(),
                                    ForkPoint {
                                        message_idx: prior_thread.len(),
                                        run_id: Some(run_id),
                                        created_at_ms: crate::runtime::authority::now_ms(),
                                    },
                                );
                            }
                            // Also record the flow_run_id in the child session's
                            // flow_run_ids for panel discovery.
                            if !flow_run_id.is_empty() {
                                self.authority
                                    .attach_flow_run_to_session(&child_sid, flow_run_id.clone());
                            }
                            child_sid
                        });
                    // Flow node sub-runs are routed to the child session when present,
                    // otherwise fall back to the parent session.
                    let flow_session_id = maybe_child_session_id
                        .clone()
                        .or_else(|| maybe_session_id.clone());

                    let flow_ctx = RunContext {
                        space_id: space,
                        workspace_root: workspace_root.clone(),
                        flow_id: Some(fid.clone()),
                        flow_run_id: Some(flow_run_id.clone()),
                        session_id: flow_session_id,
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
                        .insert(flow_run_id.clone(), flow_ctx.clone());
                    // Record the agent-run id so FlowRunApprove/FlowRunRequestChanges
                    // can emit flow.agent.notify events back to the original subscription.
                    self.flow_run_to_agent_run
                        .lock()
                        .insert(flow_run_id, run_id);

                    // Spawn ReactLoops for ALL flow entry nodes.  Crony is the
                    // supervisor and runs in the main loop with its own system
                    // prompt; every flow agent (including the first) runs in its
                    // own spawned ReactLoop so it appears in the Flow thread.
                    for ctx in entry_contexts.into_iter() {
                        let agent_id = ctx.owner.clone();
                        ar.spawn_agent(flow_ctx.clone(), agent_id, ctx);
                    }

                    // entry_sys is intentionally None: Crony should use its own
                    // system.md (loaded via preloaded_chat_agent_def) rather than
                    // the flow entry node's rendered invocation message.
                    (None, Some(flow_ctx), entry_node_id)
                } else {
                    (None, None, String::new())
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
                            // Use the supervision task's own flow_run_id as the
                            // authoritative run id.  evt.run_id can be "" when an
                            // agent was spawned via a code path that had flow_run_id
                            // = None (e.g. the invoke_flow fallback spawn_fn_sup).
                            let effective_run_id: String = fctx
                                .flow_run_id
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(&evt.run_id)
                                .to_owned();
                            match fctx
                                .flow_runtime
                                .as_ref()
                                .unwrap()
                                .on_document_submitted(
                                    &effective_run_id,
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
                                            // Human review pending — emit a lightweight
                                            // flow.agent.notify event on the original agent-run
                                            // subscription so the browser shows a chat notification
                                            // without an LLM turn.
                                            let port = inv_ctx
                                                .trigger
                                                .approved_port
                                                .as_deref()
                                                .unwrap_or("?");
                                            let producer =
                                                inv_ctx.trigger.from_node.as_deref().unwrap_or("?");
                                            let msg = format!(
                                                "📋 **{producer}** submitted **{port}** for your review"
                                            );
                                            info!(
                                                agent_run_id = %run_id,
                                                producer,
                                                port,
                                                "supervision: emitting review-ready notification"
                                            );
                                            sup_services.authority.emit_for_run(
                                                run_id,
                                                crate::protocol::events::RuntimeEventPayload::Raw {
                                                    data: serde_json::json!({
                                                        "event": "flow.agent.notify",
                                                        "kind": "info",
                                                        "message": msg,
                                                    }),
                                                },
                                            );
                                        } else {
                                            info!(
                                                agent_id,
                                                node_id = %inv_ctx.node_id,
                                                "supervision: spawning downstream agent"
                                            );
                                            sup_ar.spawn_agent(fctx.clone(), agent_id, inv_ctx);
                                        }
                                    }
                                    // Forward the flow.run.changed notification onto the
                                    // agent-run's subscription bus ("run:{run_id}"), which
                                    // the StartRun fan-out task IS subscribed to and which
                                    // routes events to the browser.  The FlowRuntime's
                                    // internal emit fires on "flow:flow.run.changed" which
                                    // has no active subscribers, so without this the
                                    // FlowDocReviewPanel never learns about the new review.
                                    sup_services.authority.emit_for_run(
                                        run_id,
                                        crate::protocol::events::RuntimeEventPayload::Raw {
                                            data: serde_json::json!({
                                                "event": "flow.run.changed",
                                                "payload": serde_json::json!({ "run_id": &effective_run_id }).to_string()
                                            }),
                                        },
                                    );
                                    info!(
                                        agent_run_id = %run_id,
                                        flow_run_id = %effective_run_id,
                                        "supervision: emitted flow.run.changed event"
                                    );
                                    // Append ProducedDoc to the agent's run (task 14.5).
                                    if let Ok(doc_run_uuid) = Uuid::parse_str(&evt.run_id) {
                                        let doc_run_id = RunId(doc_run_uuid);
                                        sup_services.authority.append_produced_doc(
                                            doc_run_id,
                                            crate::runtime::state::ProducedDoc {
                                                doc_type: evt.doc_type.clone(),
                                                path: evt.relative_path.clone(),
                                                revision: evt.revision,
                                            },
                                        );
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, run_id = %effective_run_id, "supervision: on_document_submitted failed");
                                    // Cycle limit exceeded or other terminal error — fail the run.
                                    if e.to_string().contains("cycle limit exceeded") {
                                        let _ = sup_services.authority.fail_run(
                                            RunId(
                                                Uuid::parse_str(&effective_run_id)
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
                    // No flow context — drain the doc channel and record produced docs.
                    let authority_doc = self.authority.clone();
                    tokio::spawn(async move {
                        while let Some(evt) = doc_rx.recv().await {
                            info!(doc_type = %evt.doc_type, "supervision (no flow): document submitted");
                            // Append ProducedDoc to the owning run (task 14.5).
                            if let Ok(doc_run_uuid) = Uuid::parse_str(&evt.run_id) {
                                let doc_run_id = RunId(doc_run_uuid);
                                authority_doc.append_produced_doc(
                                    doc_run_id,
                                    crate::runtime::state::ProducedDoc {
                                        doc_type: evt.doc_type.clone(),
                                        path: evt.relative_path.clone(),
                                        revision: evt.revision,
                                    },
                                );
                            }
                        }
                    });
                }

                // Build the HostCapabilityDispatcher for the entry agent.
                let mut cap_builder = HostCapabilityDispatcher::builder();
                cap_builder.register_shell(Arc::new(LocalShell::new(&workspace_root)), false);
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
                        .map(|c| c.flow_run_id.clone().unwrap_or_else(|| run_id.to_string()))
                        .unwrap_or_else(|| run_id.to_string());
                    cap_builder.register_submit_document(
                        workspace_root.clone(),
                        fid.clone(),
                        flow_run_id_for_tool,
                        entry_node_id.clone(), // actual node_id (e.g. "pm-design")
                        doc_tx.clone(),
                        self.workspace_cache_dir.clone(),
                    );
                }
                cap_builder.register_search(workspace_root.clone());
                cap_builder.register_git(workspace_root.clone());

                // Register flow.* tools for the chat session when a flow context
                // is available. The session id (if any) is registered so human-review
                // notifications can route back to this session.
                //
                // Also holds the shared FlowRuntime for the supervisor's invoke_flow
                // tool; set in the non-flow else branch below.
                let mut supervisor_flow_rt = None;
                // Captured from the non-flow else-branch so register_invoke_flow
                // gets the full spawn_fn that correctly creates per-flow supervision
                // tasks (with the right flow_run_id / doc_tx) rather than the simple
                // fallback that ignores flow_run_id.
                let mut invoke_flow_spawn_fn: Option<SpawnAgentFn> = None;
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
                        .get_or_create(&workspace_root, self.workspace_cache_dir.as_deref())
                        .await;
                    supervisor_flow_rt = Some(shared_rt.clone());
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
                    let session_id_for_spawn = maybe_session_id.clone();
                    // Per-run doc_tx map: lazily create supervision task on first
                    // spawn for each flow_run_id.
                    let run_doc_txs: Arc<
                        Mutex<HashMap<String, tokio::sync::mpsc::Sender<DocumentSubmitted>>>,
                    > = Arc::new(Mutex::new(HashMap::new()));
                    let spawn_fn: SpawnAgentFn = Arc::new(move |flow_run_id, agent_id, inv_ctx| {
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
                                let sup_workspace_root = workspace_root_for_spawn.clone();
                                let sup_base_url = base_url_for_spawn.clone();
                                let sup_api_key = api_key_for_spawn.clone();
                                let sup_model = model_for_spawn.clone();
                                let sup_provider_kind = provider_kind_for_spawn.clone();
                                let sup_sandbox = sandbox_for_spawn.clone();
                                let sup_wcd = wcd_for_spawn.clone();
                                let sup_services = services_for_spawn.clone();
                                let sup_ar = ar_for_spawn.clone();
                                let sup_session_id = session_id_for_spawn.clone();
                                let tx_clone = tx.clone();
                                tokio::spawn(async move {
                                    while let Some(evt) = rx.recv().await {
                                        info!(
                                            run_id = %evt.run_id,
                                            agent_id = %evt.agent_id,
                                            doc_type = %evt.doc_type,
                                            "supervision(chat): document submitted"
                                        );
                                        let flow_id = match sup_flow_rt.get_run(&sup_flow_run_id) {
                                            Some(s) => s.flow_id.clone(),
                                            None => {
                                                warn!(run_id = %evt.run_id, "supervision(chat): run not found");
                                                continue;
                                            }
                                        };
                                        let flow_def = match sup_services
                                            .flow_registry
                                            .load_flow_def(&flow_id, &sup_workspace_root)
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
                                            session_id: sup_session_id.clone(),
                                            flow_runtime: Some(sup_flow_rt.clone()),
                                            doc_tx: tx_clone.clone(),
                                            llm_config: LlmConfig::from_payload_fields(
                                                &sup_provider_kind,
                                                sup_base_url.clone(),
                                                sup_api_key.clone(),
                                                sup_model.clone(),
                                            ),
                                            sandbox_tier: match &sup_sandbox {
                                                Some(p) => SandboxTier::Sandboxed(p.clone()),
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
                                                    let next_agent = inv_ctx.owner.clone();
                                                    if next_agent == "human" {
                                                        if let Some(sid) = sup_flow_rt
                                                            .lookup_chat_session(&sup_flow_run_id)
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
                                                if e.to_string().contains("cycle limit exceeded") {
                                                    let _ = sup_authority.fail_run(
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
                            session_id: session_id_for_spawn.clone(),
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
                    });
                    // Save a clone for register_invoke_flow so the supervisor can
                    // start new flows with proper per-run supervision contexts.
                    invoke_flow_spawn_fn = Some(spawn_fn.clone());
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

                    // On first access after app restart, restore session routing
                    // and notify the session about any pending doc reviews.
                    if is_new {
                        let all_runs = shared_rt.list_runs();

                        // Seed authority flow_sessions from persisted originating_session_id
                        // so tool-approval routing works correctly after restart.
                        let pairs = all_runs.iter().filter_map(|r| {
                            r.originating_session_id
                                .as_ref()
                                .map(|sid| (r.run_id.clone(), sid.clone()))
                        });
                        self.authority.seed_flow_sessions(pairs);

                        // Emit a lightweight event if this session has paused flow runs
                        // with InReview ports so the frontend can fetch them without
                        // spawning a heavyweight LLM turn.
                        if let Some(ref sid) = maybe_session_id {
                            let has_pending = all_runs.iter().any(|r| {
                                r.originating_session_id.as_deref() == Some(sid.0.as_str())
                                    && r.status == crate::flow::runtime::FlowRunStatus::Paused
                                    && r.node_states.values().any(|ns| {
                                        ns.ports.values().any(|&s| {
                                            s == crate::flow::runtime::PortStatus::InReview
                                        })
                                    })
                            });
                            if has_pending {
                                self.authority.emit_for_run(
                                    run_id,
                                    crate::protocol::events::RuntimeEventPayload::Raw {
                                        data: serde_json::json!({
                                            "event": "session.pending_actions_ready",
                                            "session_id": sid.0,
                                        }),
                                    },
                                );
                            }
                        }
                    }
                }

                // Use the pre-loaded agent def (always Crony for user-initiated turns)
                // for tool allow-list, system prompt and supervisor tool registration.
                // entry_system_prompt from the flow's first node takes precedence when
                // set; this def fills the gap when it is None.
                let chat_agent_def = preloaded_chat_agent_def;

                if let Some(ref def) = chat_agent_def {
                    if !def.tools.is_empty() {
                        cap_builder.set_allowed_tools(def.tools.clone());
                    }
                }

                // Register supervisor tools (invoke_agent + invoke_flow) when
                // the chat agent is a Supervisor (e.g. the Crony builtin).
                // These tools let the supervisor delegate to specialist agents
                // and block on flow-run completion.
                if matches!(
                    chat_agent_def.as_ref().map(|d| d.kind),
                    Some(agent_loader::AgentKind::Supervisor)
                ) {
                    let authority_sup = self.authority.clone();
                    let services_sup = Arc::clone(&self.services);
                    let sandbox_sup = self.sandbox_policy.clone();
                    let wcd_sup = self.workspace_cache_dir.clone();
                    let workspace_root_sup = workspace_root.clone();
                    let maybe_session_id_sup = maybe_session_id.clone();
                    let doc_tx_sup = doc_tx.clone();
                    // Use whichever flow runtime is available: non-flow chat sets
                    // supervisor_flow_rt in the else branch; flow-context turns expose
                    // it through maybe_flow_ctx.
                    let effective_sup_flow_rt = supervisor_flow_rt
                        .clone()
                        .or_else(|| maybe_flow_ctx.as_ref().and_then(|c| c.flow_runtime.clone()));
                    let provider_kind_sup = provider_kind.clone();
                    let base_url_sup = base_url.clone();
                    let api_key_sup = api_key.clone();
                    let model_sup = model.clone();

                    let sup_run_ctx = RunContext {
                        space_id: space,
                        workspace_root: workspace_root_sup.clone(),
                        flow_id: None,
                        flow_run_id: None,
                        session_id: maybe_session_id_sup.clone(),
                        doc_tx: doc_tx_sup.clone(),
                        flow_runtime: effective_sup_flow_rt.clone(),
                        llm_config: LlmConfig::from_payload_fields(
                            &provider_kind_sup,
                            base_url_sup.clone(),
                            api_key_sup.clone(),
                            model_sup.clone(),
                        ),
                        sandbox_tier: match &sandbox_sup {
                            Some(p) => SandboxTier::Sandboxed(p.clone()),
                            None => SandboxTier::Trusted,
                        },
                        workspace_cache_dir: wcd_sup.clone(),
                    };

                    let invoke_agent_spawn: Arc<
                        dyn Fn(
                                RunContext,
                                String,
                                String,
                                tokio::sync::oneshot::Sender<crate::agent_loop::tools::AgentResult>,
                            ) + Send
                            + Sync
                            + 'static,
                    > = Arc::new(move |child_ctx, agent_id, goal, tx| {
                        let services = Arc::clone(&services_sup);
                        let authority = authority_sup.clone();
                        let middleware = build_middleware_chain(authority.clone());
                        tokio::spawn(async move {
                            let child_run_id = match authority.start_run_with_session(
                                child_ctx.space_id,
                                None,
                                serde_json::json!({ "agent_name": &agent_id }),
                                child_ctx.session_id.clone(),
                            ) {
                                Ok(id) => id,
                                Err(e) => {
                                    let _ = tx.send(crate::agent_loop::tools::AgentResult {
                                        success: false,
                                        output: serde_json::Value::Null,
                                        error: Some(format!("failed to start child run: {e}")),
                                    });
                                    return;
                                }
                            };
                            let agent_def =
                                agent_loader::load_agent(&child_ctx.workspace_root, &agent_id)
                                    .await;
                            let inv_ctx = crate::flow::runtime::InvocationContext::build(
                                &agent_id,
                                &agent_id,
                                crate::flow::runtime::InvocationTrigger {
                                    kind: "supervisor_invoke".into(),
                                    from_node: None,
                                    approved_port: None,
                                    reviewer_doc_path: None,
                                },
                                vec![],
                                vec![],
                            );
                            let system_message =
                                crate::runtime::agent_runner::render_system_message(&inv_ctx);
                            let system_message = if agent_def.system_prompt.is_empty() {
                                system_message
                            } else {
                                format!("{}\n\n---\n\n{}", agent_def.system_prompt, system_message)
                            };
                            let child_model = {
                                let parent_model = match &child_ctx.llm_config {
                                    LlmConfig::OpenAi { model, .. }
                                    | LlmConfig::Anthropic { model, .. }
                                    | LlmConfig::Copilot { model, .. } => model.clone(),
                                };
                                if agent_def.llm_model.is_empty() {
                                    parent_model
                                } else {
                                    agent_def.llm_model.clone()
                                }
                            };
                            let child_llm_config = match child_ctx.llm_config.clone() {
                                LlmConfig::OpenAi {
                                    base_url, api_key, ..
                                } => LlmConfig::OpenAi {
                                    base_url,
                                    api_key,
                                    model: child_model.clone(),
                                },
                                LlmConfig::Anthropic {
                                    base_url, api_key, ..
                                } => LlmConfig::Anthropic {
                                    base_url,
                                    api_key,
                                    model: child_model.clone(),
                                },
                                LlmConfig::Copilot {
                                    github_token,
                                    base_url,
                                    ..
                                } => LlmConfig::Copilot {
                                    github_token,
                                    base_url,
                                    model: child_model.clone(),
                                },
                            };
                            let llm = match services.llm_factory.build(&child_llm_config).await {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = authority.fail_run(child_run_id, e.to_string());
                                    let _ = tx.send(crate::agent_loop::tools::AgentResult {
                                        success: false,
                                        output: serde_json::Value::Null,
                                        error: Some(format!("llm build failed: {e}")),
                                    });
                                    return;
                                }
                            };
                            let child_cap = services
                                .capability_factory
                                .build(&child_ctx.workspace_root, child_ctx.sandbox_tier.clone());
                            let child_tools = Arc::new(child_cap.build());
                            let cfg = LoopConfig {
                                model: child_model,
                                system_prompt: Some(system_message),
                                user_input: goal,
                                max_turns: 99999,
                                temperature: None,
                                reasoning_effort: None,
                                llm,
                                tools: child_tools,
                                thinking: None,
                                initial_thread: None,
                                session_id: None,
                                reflection: agent_def.reflection.clone(),
                                critic: agent_def.critic.clone(),
                                write_namespace: None,
                                memory_manager: None,
                                middleware,
                                agent_name: Some(agent_id.clone()),
                            };
                            let result = ReactLoop::new(authority.clone(), child_run_id, cfg)
                                .run()
                                .await;
                            let agent_result = match result {
                                Ok(()) => crate::agent_loop::tools::AgentResult {
                                    success: true,
                                    output: serde_json::json!({
                                        "run_id": child_run_id.to_string()
                                    }),
                                    error: None,
                                },
                                Err(e) => crate::agent_loop::tools::AgentResult {
                                    success: false,
                                    output: serde_json::Value::Null,
                                    error: Some(e.to_string()),
                                },
                            };
                            let _ = tx.send(agent_result);
                        });
                    });

                    register_invoke_agent(
                        &mut cap_builder,
                        self.authority.clone(),
                        run_id,
                        sup_run_ctx.clone(),
                        invoke_agent_spawn,
                    );

                    if let Some(ref flow_rt) = effective_sup_flow_rt {
                        let flow_rt_sup = Arc::clone(flow_rt);
                        // Use the big spawn_fn from the non-flow else-branch when
                        // available — it correctly builds per-flow RunContexts with
                        // the actual flow_run_id and lazily starts supervision tasks.
                        // Fall back to a simple version only when a bound flow context
                        // is already active (invoke_flow would be unusual there anyway).
                        let spawn_fn_sup: SpawnAgentFn = if let Some(ref sfn) = invoke_flow_spawn_fn
                        {
                            sfn.clone()
                        } else {
                            let ar_s = ar.clone();
                            let ctx_s = sup_run_ctx.clone();
                            Arc::new(move |flow_run_id: String, agent_id, inv_ctx| {
                                // Propagate the actual flow_run_id so that agents spawned
                                // for sub-flows have the correct run id when registering
                                // submit_document.  Without this they inherit None from
                                // sup_run_ctx and emit DocumentSubmitted { run_id: "" }.
                                let ctx = RunContext {
                                    flow_run_id: Some(flow_run_id),
                                    ..ctx_s.clone()
                                };
                                ar_s.spawn_agent(ctx, agent_id, inv_ctx);
                            })
                        };
                        let flow_rt_poll = Arc::clone(flow_rt);
                        let flow_completion_fn: Arc<
                            dyn Fn(
                                    String,
                                    tokio::sync::oneshot::Sender<
                                        crate::agent_loop::tools::AgentResult,
                                    >,
                                ) + Send
                                + Sync
                                + 'static,
                        > = Arc::new(move |flow_run_id, tx| {
                            let rt = Arc::clone(&flow_rt_poll);
                            tokio::spawn(async move {
                                loop {
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    if let Some(state) = rt.get_run(&flow_run_id) {
                                        if state.status.is_terminal() {
                                            let success = state.status
                                                == crate::flow::runtime::FlowRunStatus::Completed;
                                            let _ =
                                                tx.send(crate::agent_loop::tools::AgentResult {
                                                    success,
                                                    output: serde_json::json!({
                                                        "flow_run_id": flow_run_id,
                                                        "status": format!("{:?}", state.status),
                                                    }),
                                                    error: if success {
                                                        None
                                                    } else {
                                                        state.failure_reason.clone()
                                                    },
                                                });
                                            return;
                                        }
                                    } else {
                                        let _ = tx.send(crate::agent_loop::tools::AgentResult {
                                            success: false,
                                            output: serde_json::Value::Null,
                                            error: Some(format!(
                                                "flow run '{flow_run_id}' not found"
                                            )),
                                        });
                                        return;
                                    }
                                }
                            });
                        });
                        let invoke_flow_desc = build_invoke_flow_description(&workspace_root).await;
                        register_invoke_flow(
                            &mut cap_builder,
                            invoke_flow_desc,
                            self.authority.clone(),
                            run_id,
                            flow_rt_sup,
                            workspace_root.clone(),
                            spawn_fn_sup,
                            flow_completion_fn,
                            // Bind the new flow run to the child session (or parent
                            // session) so the frontend's Flow thread subscription
                            // receives flow.run.changed and run_status events.
                            child_session_id
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                                .or_else(|| maybe_session_id.as_ref().map(|s| s.0.clone())),
                        );
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
                        let block =
                            build_workspace_injection_block(&workspace_root, &tool_names_sorted);
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
                    let prompt_ctx = crate::runtime::prompt::VarContext::builder()
                        .workspace_root(workspace_root.clone())
                        .agent_name(agent_name.clone())
                        .user_vars(agent_vars)
                        .agents(workspace_agent_names)
                        .build();
                    crate::runtime::prompt::render(&tmpl, &prompt_ctx)
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
                                match copilot_auth::exchange_for_copilot_token(&http, github_token)
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
                        .or_else(|| Some(crate::crony::prompts::SYSTEM_PROMPT.to_string()));

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
                                    let count =
                                        authority.session_thread(sid).map(|t| t.len()).unwrap_or(0);
                                    let mem_key = format!("compaction/{count}");
                                    let _ = authority.put_memory(
                                        ns.clone(),
                                        crate::runtime::state::MemoryEntry {
                                            key: mem_key.clone(),
                                            value: serde_json::json!({ "summary": summary }),
                                            updated_at_ms: crate::runtime::authority::now_ms(),
                                        },
                                    );
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

                    // Tell Crony which flow was auto-started so it knows to
                    // monitor rather than restart it with invoke_flow.
                    let user_input =
                        if let (Some(ref fid), Some(ref fctx)) = (&flow_id_opt, &maybe_flow_ctx) {
                            let frid = fctx.flow_run_id.as_deref().unwrap_or("?");
                            format!(
                                "[Flow '{fid}' started (run_id: {frid}). \
                             Flow agents are executing — use flow_status to track progress \
                             and flow_approve/flow_request_changes to manage reviews. \
                             Summarise for the user when complete.]\n\n{user_input}"
                            )
                        } else {
                            user_input
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
                        // Crony (the supervisor) always tracks its session so
                        // the conversation history is preserved across turns.
                        session_id: maybe_session_id.clone(),
                        reflection: chat_agent_def.as_ref().and_then(|d| d.reflection.clone()),
                        critic: chat_agent_def.as_ref().and_then(|d| d.critic.clone()),
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
                        agent_name: None,
                    };
                    let result = ReactLoop::new(authority.clone(), run_id, cfg).run().await;
                    info!(%run_id, ok = result.is_ok(), "react_loop: finished");
                    if let Err(e) = &result {
                        info!(%run_id, error = %e, "react_loop: failed with error");
                    }
                    // Persist Crony's conversation history (supervisor turns belong
                    // in the session; only spawned flow-agent loops must not write here).
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
                    run_id: run_id.to_string(),
                    subscription: sub_id,
                }
            }
            Err(e) => ControlResponse::Err {
                error: authority_err_to_control(e, Some(&space_id), None),
            },
        }
    }
}
