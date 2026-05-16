//! [`AgentRunner`] — replaces the free functions `spawn_agent_loop` and
//! `spawn_chat_turn` from `handler.rs`.
//!
//! The runner holds an `Arc<RuntimeServices>` so it can access the LLM factory,
//! capability factory, authority, and flow registry without requiring them to be
//! passed as arguments.

use std::sync::Arc;

use tracing::{info, warn};

use crate::agent_loop::{LoopConfig, ReactLoop, ToolDispatcher};
use crate::capability::agent_loader;
use crate::capability::flow_tools::{register_flow_tools, register_submit_review, SpawnAgentFn};
use crate::flow::runtime::InvocationContext;
use crate::llm::{CapabilityResolver, LlmConfig};
use crate::runtime::middleware::{
    LlmDurationStore, MiddlewareChain, TimingMiddleware, TokenAccumulatorMiddleware,
    ToolDurationStore, TraceEmitterMiddleware,
};
use crate::runtime::run_context::RunContext;
use crate::runtime::services::RuntimeServices;
use crate::runtime::state::SessionId;

// ── AgentRunner ───────────────────────────────────────────────────────────────

/// Owns the logic for spawning `ReactLoop` tasks for agent and chat invocations.
///
/// Callers that previously called `spawn_agent_loop` / `spawn_chat_turn` should
/// hold an `Arc<AgentRunner>` and call `spawn_agent` / `spawn_chat` instead.
#[derive(Clone)]
pub struct AgentRunner {
    services: Arc<RuntimeServices>,
}

impl AgentRunner {
    pub fn new(services: Arc<RuntimeServices>) -> Self {
        Self { services }
    }

    /// Spawn a `ReactLoop` for one agent invocation within a flow run.
    ///
    /// Creates its own `authority_run_id` so the authority's lifecycle
    /// tracking does not interfere with the parent flow run.
    pub fn spawn_agent(&self, run_ctx: RunContext, agent_id: String, inv_ctx: InvocationContext) {
        let services = Arc::clone(&self.services);
        let authority = services.authority.clone();

        let run_id = match authority.start_run(run_ctx.space_id, None, serde_json::json!({})) {
            Ok(id) => id,
            Err(e) => {
                warn!(agent_id, error = %e, "agent_runner: authority.start_run failed");
                return;
            }
        };

        if let Some(ref frid) = run_ctx.flow_run_id {
            authority.set_run_flow_id(run_id, frid.clone());
        }

        tokio::spawn(async move {
            let agent_def = agent_loader::load_agent(&run_ctx.workspace_root, &agent_id).await;

            // Build system prompt: agent persona + flow invocation rendering.
            let inv_system_message = render_system_message(&inv_ctx);
            let agent_system_prompt_raw = if agent_def.system_prompt.is_empty() {
                inv_system_message
            } else {
                format!(
                    "{}\n\n---\n\n{}",
                    agent_def.system_prompt, inv_system_message
                )
            };
            let prompt_ctx = crate::runtime::prompt::VarContext::builder()
                .workspace_root(run_ctx.workspace_root.clone())
                .agent_name(agent_id.clone())
                .user_vars(agent_def.vars.clone())
                .build();
            let system_message =
                crate::runtime::prompt::render(&agent_system_prompt_raw, &prompt_ctx);

            // Model override from agent YAML.
            let model = match &run_ctx.llm_config {
                LlmConfig::OpenAi { model, .. }
                | LlmConfig::Anthropic { model, .. }
                | LlmConfig::Copilot { model, .. } => {
                    if agent_def.llm_model.is_empty() {
                        model.clone()
                    } else {
                        agent_def.llm_model.clone()
                    }
                }
            };

            let runner_role = agent_def.kind.as_str();

            // ── Build capability dispatcher ───────────────────────────────
            let mut cap_builder = services
                .capability_factory
                .build(&run_ctx.workspace_root, run_ctx.sandbox_tier.clone());

            let store = crate::capability::test_runner::LastReportStore::new();
            let flow_run_id = run_ctx.flow_run_id.clone().unwrap_or_default();
            let flow_id = run_ctx.flow_id.clone().unwrap_or_default();
            cap_builder.register_test_runner(
                run_ctx.workspace_root.clone(),
                store,
                flow_run_id.clone(),
                runner_role,
            );
            cap_builder.register_submit_document(
                run_ctx.workspace_root.clone(),
                flow_id.clone(),
                flow_run_id.clone(),
                agent_id.clone(),
                run_ctx.doc_tx.clone(),
            );
            cap_builder.register_search(run_ctx.workspace_root.clone());
            cap_builder.register_git(run_ctx.workspace_root.clone());

            if inv_ctx.trigger.kind == "reviewer_invocation" {
                if let (Some(producer_node), Some(port)) = (
                    inv_ctx.trigger.from_node.clone(),
                    inv_ctx.trigger.approved_port.clone(),
                ) {
                    let services_spawn = Arc::clone(&services);
                    let run_ctx_spawn = run_ctx.clone();
                    let spawn_fn: SpawnAgentFn =
                        Arc::new(move |_flow_run_id, agent_id2, inv_ctx2| {
                            let runner = AgentRunner::new(Arc::clone(&services_spawn));
                            runner.spawn_agent(run_ctx_spawn.clone(), agent_id2, inv_ctx2);
                        });
                    if let Some(flow_rt) = &run_ctx.flow_runtime {
                        register_submit_review(
                            &mut cap_builder,
                            Arc::clone(flow_rt),
                            run_ctx.workspace_root.clone(),
                            agent_id.clone(),
                            producer_node,
                            port,
                            flow_run_id.clone(),
                            spawn_fn,
                        );
                    }
                }
            }

            if !agent_def.tools.is_empty() {
                cap_builder.set_allowed_tools(agent_def.tools.clone());
            }

            let tools = Arc::new(cap_builder.build());

            // Workspace injection block.
            let system_message = if agent_def.inject_workspace {
                let defs = tools.definitions();
                let mut tool_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
                tool_names.sort_unstable();
                let tools_line = tool_names.join(", ");
                let block = format!(
                    "\n---\nWorkspace: `{}`\nTools available: {}\n\
                     Use these tools to verify facts about the codebase. Never guess at structure.",
                    run_ctx.workspace_root.display(),
                    tools_line
                );
                format!("{system_message}{block}")
            } else {
                system_message
            };

            // ── Build LLM provider ────────────────────────────────────────
            // Resolve thinking config before building the provider (Anthropic only).
            let thinking_config = if let LlmConfig::Anthropic {
                ref base_url,
                ref api_key,
                model: ref mdl,
            } = run_ctx.llm_config
            {
                let caps = CapabilityResolver::resolve(mdl, base_url, api_key.as_deref()).await;
                caps.thinking_config()
            } else {
                None
            };

            // Build the LLM config with the effective model.
            let effective_llm_config = apply_model_override(run_ctx.llm_config.clone(), &model);
            let llm = match services.llm_factory.build(&effective_llm_config).await {
                Ok(p) => p,
                Err(e) => {
                    warn!(agent_id, error = %e, "agent_runner: llm_factory.build failed");
                    let _ = authority.fail_run(run_id, e.to_string());
                    return;
                }
            };

            let cfg = LoopConfig {
                model: model.clone(),
                system_prompt: Some(system_message),
                user_input: "Continue with your assigned task as described above.".to_owned(),
                max_turns: 99999,
                temperature: None,
                reasoning_effort: None,
                llm,
                tools,
                thinking: thinking_config,
                initial_thread: None,
                session_id: None,
                reflection: None,
                write_namespace: None,
                memory_manager: None,
                middleware: build_middleware_chain(authority.clone()),
            };

            let result = ReactLoop::new(authority.clone(), run_id, cfg).run().await;
            info!(agent_id, %run_id, ok = result.is_ok(), "agent_runner: agent loop finished");
            if let Err(e) = result {
                info!(agent_id, %run_id, error = %e, "agent_runner: agent loop failed");
            }
        });
    }

    /// Spawn a single-turn `ReactLoop` for a chat session notification.
    ///
    /// Loads the session thread, prepends `message` as a user message,
    /// and runs one ReactLoop turn. The updated thread is flushed back for
    /// the next turn.
    pub fn spawn_chat(&self, run_ctx: RunContext, session_id: String, message: String) {
        let services = Arc::clone(&self.services);
        let authority = services.authority.clone();
        let sid = SessionId(session_id);

        let run_id = match authority.start_run_with_session(
            run_ctx.space_id,
            None,
            serde_json::json!({}),
            Some(sid.clone()),
        ) {
            Ok(id) => id,
            Err(e) => {
                warn!(error = %e, "agent_runner: spawn_chat authority.start_run failed");
                return;
            }
        };

        if let Some(ref frid) = run_ctx.flow_run_id {
            if !frid.is_empty() {
                authority.set_run_flow_id(run_id, frid.clone());
            }
        }

        // Bind session so future calls can route notifications back.
        if let Some(ref frid) = run_ctx.flow_run_id {
            authority.bind_session(frid, &sid.0);
        }

        tokio::spawn(async move {
            let thread = if let Some(ref cache_dir) = run_ctx.workspace_cache_dir {
                crate::runtime::chat_store::ChatStore::new(cache_dir).load_history(&sid)
            } else {
                authority.session_thread(&sid).unwrap_or_default()
            };
            let prior_thread_len = thread.len();

            let chat_agent_def =
                agent_loader::load_agent(&run_ctx.workspace_root, "__chat__").await;

            let system_prompt = if chat_agent_def.system_prompt.is_empty() {
                None
            } else {
                Some(chat_agent_def.system_prompt.clone())
            };

            // ── Build capability dispatcher ───────────────────────────────
            let mut cap_builder = services
                .capability_factory
                .build(&run_ctx.workspace_root, run_ctx.sandbox_tier.clone());
            cap_builder.register_search(run_ctx.workspace_root.clone());
            cap_builder.register_git(run_ctx.workspace_root.clone());

            if let Some(flow_rt) = &run_ctx.flow_runtime {
                let services_spawn = Arc::clone(&services);
                let run_ctx_spawn = run_ctx.clone();
                let spawn_fn: SpawnAgentFn = Arc::new(move |_flow_run_id, agent_id, inv_ctx| {
                    let runner = AgentRunner::new(Arc::clone(&services_spawn));
                    runner.spawn_agent(run_ctx_spawn.clone(), agent_id, inv_ctx);
                });
                register_flow_tools(
                    &mut cap_builder,
                    Arc::clone(flow_rt),
                    run_ctx.workspace_root.clone(),
                    sid.0.clone(),
                    spawn_fn,
                );
            }

            if !chat_agent_def.tools.is_empty() {
                cap_builder.set_allowed_tools(chat_agent_def.tools.clone());
            }

            let tools = Arc::new(cap_builder.build());

            let model = match &run_ctx.llm_config {
                LlmConfig::OpenAi { model, .. }
                | LlmConfig::Anthropic { model, .. }
                | LlmConfig::Copilot { model, .. } => {
                    if chat_agent_def.llm_model.is_empty() {
                        model.clone()
                    } else {
                        chat_agent_def.llm_model.clone()
                    }
                }
            };

            let effective_llm_config = apply_model_override(run_ctx.llm_config.clone(), &model);
            let llm = match services.llm_factory.build(&effective_llm_config).await {
                Ok(p) => p,
                Err(e) => {
                    warn!(error = %e, "agent_runner: spawn_chat llm_factory.build failed");
                    let _ = authority.fail_run(run_id, e.to_string());
                    return;
                }
            };

            let cfg = LoopConfig {
                model: model.clone(),
                system_prompt,
                user_input: message,
                max_turns: 99999,
                temperature: None,
                reasoning_effort: None,
                llm,
                tools,
                thinking: None,
                initial_thread: Some(thread),
                session_id: Some(sid.clone()),
                reflection: None,
                write_namespace: None,
                memory_manager: None,
                middleware: build_middleware_chain(authority.clone()),
            };

            let result = ReactLoop::new(authority.clone(), run_id, cfg).run().await;
            if let Err(e) = result {
                warn!(error = %e, "agent_runner: chat loop failed");
            }

            // Flush updated thread.
            if let Some(updated_thread) = authority.session_thread(&sid) {
                let _ = authority.flush_thread(&sid, updated_thread.clone());
                if let Some(ref cache_dir) = run_ctx.workspace_cache_dir {
                    if updated_thread.len() > prior_thread_len {
                        let store = crate::runtime::chat_store::ChatStore::new(cache_dir);
                        let _ = store.append_turns(&sid, &updated_thread[prior_thread_len..]);
                    }
                }
            }
        });
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Render the system message that gets prepended to an agent's initial history.
///
/// This is a pure function of the `InvocationContext` fields; no side-effects.
/// Centralised here so the rendering boundary is co-located with the
/// `AgentRunner` that consumes it.
pub fn render_system_message(inv_ctx: &InvocationContext) -> String {
    let node_id = &inv_ctx.node_id;
    let owner = &inv_ctx.owner;
    let trigger = &inv_ctx.trigger;

    let next_task = inv_ctx
        .pending_ports
        .first()
        .map(|p| p.as_str())
        .unwrap_or("none");

    let available_summary = if inv_ctx.available_docs.is_empty() {
        "No documents have been approved yet in this run.".to_owned()
    } else {
        inv_ctx
            .available_docs
            .iter()
            .map(|d| format!("  - {} ({}, rev {})", d.path, d.doc_type, d.revision))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let pending_summary = if inv_ctx.pending_ports.is_empty() {
        "All your ports are complete.".to_owned()
    } else {
        inv_ctx
            .pending_ports
            .iter()
            .enumerate()
            .map(|(i, p)| format!("  {}. {}", i + 1, p))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let trigger_context = match (trigger.kind.as_str(), trigger.approved_port.as_deref()) {
        ("and_join", Some(port)) => format!(
            "All required inputs for node `{node_id}` have been approved. \
             Last approval: `{port}` from node `{}`.",
            trigger.from_node.as_deref().unwrap_or("?")
        ),
        ("cycle_retrigger", Some(port)) => format!(
            "Node `{node_id}` has been re-triggered by the cycle input `{port}` \
             from node `{}`.",
            trigger.from_node.as_deref().unwrap_or("?")
        ),
        ("implicit_reinvoke", Some(port)) => format!(
            "Your output `{port}` was approved. You still have pending ports — \
             please continue with the next task."
        ),
        ("rejected_requeue", Some(port)) => format!(
            "Your submission for port `{port}` was rejected with change requests. \
             Please address the reviewer feedback below and resubmit."
        ),
        ("reviewer_invocation", Some(port)) => {
            let doc_path = trigger
                .reviewer_doc_path
                .as_deref()
                .unwrap_or("<unknown path>");
            let producer = trigger.from_node.as_deref().unwrap_or("?");
            format!(
                "You have been assigned to review the document submitted by node `{producer}` \
                 at port `{port}`.\n\
                 Document path: `{doc_path}`\n\n\
                 Use the `read_file` tool to load the document, then call \
                 `flow_submit_review` with your verdict (`approve` or `reject`) \
                 and optional structured comments."
            )
        }
        _ => format!("Node `{node_id}` ({owner}) is being invoked."),
    };

    // Reviewer invocations use a compact format.
    if trigger.kind == "reviewer_invocation" {
        return format!("## FlowRuntime: Review Assignment\n\n{trigger_context}");
    }

    let feedback_section = match &inv_ctx.review_comments {
        Some(comments) if !comments.is_empty() => {
            let items = comments
                .iter()
                .map(|c| {
                    let sev = c.severity.to_uppercase();
                    match &c.suggestion {
                        Some(s) => format!("  - [{sev}] {} \u{2192} {s}", c.message),
                        None => format!("  - [{sev}] {}", c.message),
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\n### Reviewer Feedback (address before resubmitting)\n{items}")
        }
        Some(_) => "\n\n### Reviewer Feedback\nNo specific comments provided. \
            Review the document and improve quality."
            .to_owned(),
        None => String::new(),
    };

    format!(
        "## FlowRuntime: Invocation Context\n\n\
         {trigger_context}\n\n\
         ### Your Next Task\n\
         Submit a document of type: **{next_task}**\n\n\
         ### Your Pending Ports (in order)\n\
         {pending_summary}\n\n\
         ### Available Approved Documents\n\
         {available_summary}{feedback_section}\n\n\
         Proceed with your next task. Use the `submit_document` tool when ready."
    )
}

/// Produce a copy of `config` with the model field replaced by `model`.
///
/// Used to honour per-agent model overrides from agent YAML.
fn apply_model_override(config: LlmConfig, model: &str) -> LlmConfig {
    match config {
        LlmConfig::OpenAi {
            base_url, api_key, ..
        } => LlmConfig::OpenAi {
            base_url,
            api_key,
            model: model.to_owned(),
        },
        LlmConfig::Anthropic {
            base_url, api_key, ..
        } => LlmConfig::Anthropic {
            base_url,
            api_key,
            model: model.to_owned(),
        },
        LlmConfig::Copilot {
            github_token,
            base_url,
            ..
        } => LlmConfig::Copilot {
            github_token,
            base_url,
            model: model.to_owned(),
        },
    }
}

/// Construct the default middleware chain for a `ReactLoop`.
///
/// `TimingMiddleware` runs first (records start times); `TokenAccumulatorMiddleware`
/// second (updates `TurnContext.total_usage`); `TraceEmitterMiddleware` last
/// (reads both timing data and updated usage).
pub(crate) fn build_middleware_chain(
    authority: crate::runtime::authority::RuntimeAuthority,
) -> Arc<MiddlewareChain> {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::runtime::{AvailableDoc, InvocationContext, InvocationTrigger, ReviewComment};

    fn trigger(kind: &str) -> InvocationTrigger {
        InvocationTrigger {
            kind: kind.to_owned(),
            approved_port: None,
            from_node: None,
            reviewer_doc_path: None,
        }
    }

    fn trigger_with_port(kind: &str, port: &str, from_node: &str) -> InvocationTrigger {
        InvocationTrigger {
            kind: kind.to_owned(),
            approved_port: Some(port.to_owned()),
            from_node: Some(from_node.to_owned()),
            reviewer_doc_path: None,
        }
    }

    fn reviewer_trigger(port: &str, from_node: &str, doc_path: &str) -> InvocationTrigger {
        InvocationTrigger {
            kind: "reviewer_invocation".to_owned(),
            approved_port: Some(port.to_owned()),
            from_node: Some(from_node.to_owned()),
            reviewer_doc_path: Some(doc_path.to_owned()),
        }
    }

    #[test]
    fn render_fresh_start_and_join() {
        let ctx = InvocationContext {
            node_id: "rd-design".to_owned(),
            owner: "rd".to_owned(),
            trigger: trigger_with_port("and_join", "prd", "pm-design"),
            available_docs: vec![AvailableDoc {
                path: "docs/prd.md".to_owned(),
                doc_type: "prd".to_owned(),
                revision: 1,
            }],
            pending_ports: vec!["tech-spec".to_owned(), "code-description".to_owned()],
            review_comments: None,
        };
        let msg = render_system_message(&ctx);
        assert!(msg.contains("## FlowRuntime: Invocation Context"));
        assert!(msg.contains("All required inputs for node `rd-design` have been approved."));
        assert!(msg.contains("Last approval: `prd` from node `pm-design`."));
        assert!(msg.contains("Submit a document of type: **tech-spec**"));
        assert!(msg.contains("  1. tech-spec"));
        assert!(msg.contains("  2. code-description"));
        assert!(msg.contains("  - docs/prd.md (prd, rev 1)"));
    }

    #[test]
    fn render_reviewer_invocation() {
        let ctx = InvocationContext {
            node_id: "critic".to_owned(),
            owner: "critic".to_owned(),
            trigger: reviewer_trigger("prd", "pm-design", "docs/prd.md"),
            available_docs: vec![],
            pending_ports: vec![],
            review_comments: None,
        };
        let msg = render_system_message(&ctx);
        assert!(msg.starts_with("## FlowRuntime: Review Assignment"));
        assert!(msg.contains("review the document submitted by node `pm-design`"));
        assert!(msg.contains("at port `prd`"));
        assert!(msg.contains("Document path: `docs/prd.md`"));
    }

    #[test]
    fn render_rejected_requeue_with_feedback() {
        let ctx = InvocationContext {
            node_id: "rd-design".to_owned(),
            owner: "rd".to_owned(),
            trigger: trigger_with_port("rejected_requeue", "tech-spec", "critic"),
            available_docs: vec![],
            pending_ports: vec!["tech-spec".to_owned()],
            review_comments: Some(vec![ReviewComment {
                severity: "error".to_owned(),
                message: "Missing scalability section".to_owned(),
                suggestion: Some("Add a scalability section".to_owned()),
            }]),
        };
        let msg = render_system_message(&ctx);
        assert!(msg.contains("## FlowRuntime: Invocation Context"));
        assert!(msg.contains("was rejected with change requests"));
        assert!(msg.contains("### Reviewer Feedback (address before resubmitting)"));
        assert!(msg.contains("[ERROR] Missing scalability section"));
    }

    #[test]
    fn render_implicit_reinvoke() {
        let ctx = InvocationContext {
            node_id: "rd-design".to_owned(),
            owner: "rd".to_owned(),
            trigger: InvocationTrigger {
                kind: "implicit_reinvoke".to_owned(),
                approved_port: Some("tech-spec".to_owned()),
                from_node: None,
                reviewer_doc_path: None,
            },
            available_docs: vec![],
            pending_ports: vec!["code-description".to_owned()],
            review_comments: None,
        };
        let msg = render_system_message(&ctx);
        assert!(msg.contains("Your output `tech-spec` was approved."));
        assert!(msg.contains("Submit a document of type: **code-description**"));
    }

    #[test]
    fn render_no_pending_ports_shows_all_complete() {
        let ctx = InvocationContext {
            node_id: "rd-design".to_owned(),
            owner: "rd".to_owned(),
            trigger: trigger("unknown_trigger"),
            available_docs: vec![],
            pending_ports: vec![],
            review_comments: None,
        };
        let msg = render_system_message(&ctx);
        assert!(msg.contains("All your ports are complete."));
        assert!(msg.contains("Submit a document of type: **none**"));
    }
}
