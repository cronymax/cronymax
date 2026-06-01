//! Run lifecycle operation handlers: resume, swap memory, post input, resolve review.
//! CancelRun and PauseRun are one-liners left inline in mod.rs.

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};

use crate::agent_loop::tools::ToolDispatcher;
use crate::agent_loop::{
    maybe_compact, LoopConfig, ReactLoop, DEFAULT_RECENCY_TURNS, DEFAULT_THRESHOLD_PCT,
};
use crate::capability::agent_loader;
use crate::capability::dispatcher::HostCapabilityDispatcher;
use crate::capability::filesystem::{LocalFilesystem, WorkspaceScope};
use crate::capability::notify::NullNotify;
use crate::capability::shell::LocalShell;
use crate::llm::{
    copilot_auth, AnthropicConfig, AnthropicProvider, CapabilityResolver, OpenAiConfig,
    OpenAiProvider,
};
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse, ReviewDecision};
use crate::protocol::envelope::RuntimeToClient;
use crate::runtime::run_context::RunContext;
use crate::runtime::state::{PermissionState, RunStatus};

use super::helpers::{
    authority_err_to_control, build_middleware_chain, build_workspace_injection_block,
    parse_review, parse_run,
};
use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_resume_run(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ResumeRun { run_id } = req else {
            unreachable!()
        };
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
        match &run.status {
            RunStatus::Paused | RunStatus::AwaitingReview => {}
            _ => return ControlResponse::Ack, // already running or terminal
        }

        // Recover the agent id. `handle_start_run` mirrors the StartRun
        // control field into `run.spec["agent_id"]`; the typed
        // `run.agent_id` slot is only populated for persisted Agent
        // entities. Spec first, typed field next, Crony builtin last.
        let resolved_agent_id = run
            .spec
            .get("agent_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                run.agent_id
                    .as_ref()
                    .map(|a| a.to_string())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| crate::crony::CronyBuiltin::ID.to_owned());

        // Extension-provider runs cannot be resumed. Guard symmetric to
        // the one in `handle_start_run`: a run originally driven by an
        // extension AgentProvider has no resumable native state. The
        // extension's `AgentSession` lived only in the extension host
        // process plus the in-memory `AgentSessionRouter` sink — both are
        // gone once the run left `Running` (notably after a runtime
        // restart, which `RuntimeAuthority::rehydrate` pauses every
        // `Running`/`Pending` run). Falling through to the native
        // `agent_loader` reconstruction below would silently re-run the
        // turn against the workspace-default LLM under the provider's
        // name — the exact divergence `handle_start_run` rejects. Must be
        // checked before `mark_run_running` so a rejected resume leaves
        // the run `Paused` rather than orphaned in `Running`.
        //
        // The spec's `contribution_kind` is authoritative — the picker
        // recorded what was selected when the run started. Runs without
        // a recorded kind predate the contribution registry and are
        // treated as native (workspace YAML / Crony) runs.
        let persisted_kind = run
            .spec
            .get("contribution_kind")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let is_extension_run = matches!(
            persisted_kind,
            Some(crate::extensions::contributions::kind::AGENTS_PROVIDER)
        );
        if is_extension_run {
            let provider_label = self
                .services
                .extensions
                .as_ref()
                .and_then(|ext| ext.providers().get(&resolved_agent_id))
                .map(|p| format!("`{}` (from `{}`)", p.provider_id, p.owning_ext))
                .unwrap_or_else(|| format!("`{resolved_agent_id}`"));
            return ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: format!(
                        "run `{run_id}` was driven by extension provider {provider_label}; extension sessions cannot be resumed — start a new chat",
                    ),
                },
            };
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

        let workspace_root_clone = workspace_root.clone();
        let chat_agent_def =
            agent_loader::load_agent_with_builtin(&workspace_root_clone, &resolved_agent_id).await;
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
            let mut tool_names_sorted: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
            tool_names_sorted.sort_unstable();
            let block = build_workspace_injection_block(&workspace_root, &tool_names_sorted);
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
                        match copilot_auth::exchange_for_copilot_token(&http, github_token).await {
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
                let caps =
                    CapabilityResolver::resolve(&model, &base_url, effective_api_key.as_deref())
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
                critic: None,
                write_namespace: None,
                memory_manager: memory_manager.clone(),
                middleware: build_middleware_chain(authority.clone()),
                agent_name: None,
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

    pub(super) fn handle_swap_memory(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::SwapMemory {
            session_id,
            target,
            namespace_id,
        } = req
        else {
            unreachable!()
        };
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

    pub(super) fn handle_post_input(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::PostInput { run_id, payload } = req else {
            unreachable!()
        };
        self.run_op(&run_id, |a, id| a.post_input(id, payload.clone()))
    }

    pub(super) async fn handle_resolve_review(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ResolveReview {
            run_id,
            review_id,
            decision,
            notes,
        } = req
        else {
            unreachable!()
        };
        // `run_id` may be omitted by older clients; derive it from the
        // review when that happens so approval always succeeds.
        let run: crate::runtime::state::RunId = if run_id.is_empty() {
            let rev = match parse_review(&review_id) {
                Ok(r) => r,
                Err(resp) => return resp,
            };
            match self.authority.run_id_for_review(rev) {
                Some(r) => r,
                None => {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("unknown review: {review_id}"),
                        },
                    }
                }
            }
        } else {
            match parse_run(&run_id) {
                Ok(r) => r,
                Err(resp) => return resp,
            }
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
                    .load_flow_def(fctx.flow_id.as_deref().unwrap_or(""), &fctx.workspace_root)
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
                        .on_document_approved(&flow_run_id, producing_agent, port, &flow_def)
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
}
