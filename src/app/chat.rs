//! Sub-module extracted from app/mod.rs

use super::*;

// ─── Chat Submission Handler ─────────────────────────────────────────────────

/// Parse an `@agent-name` prefix from user input and look up the agent manifest.
///
/// Returns `(Some(manifest), remaining_text)` if an agent is found and enabled,
/// `(None, original_text)` otherwise.
pub(super) fn parse_agent_prefix(
    text: &str,
    registry: &crate::ai::agent::AgentRegistry,
) -> (Option<crate::ai::agent::AgentManifest>, String) {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('@') {
        // Extract agent name (everything up to the first whitespace).
        let (name, remainder) = match rest.find(char::is_whitespace) {
            Some(pos) => (&rest[..pos], rest[pos..].trim_start()),
            None => (rest, ""),
        };
        if !name.is_empty() {
            if let Some(manifest) = registry.lookup(name) {
                if manifest.agent.enabled {
                    return (Some(manifest.clone()), remainder.to_string());
                } else {
                    log::info!("Agent '{}' is disabled", name);
                }
            } else {
                log::info!("Agent '{}' not found in registry", name);
            }
        }
    }
    (None, text.to_string())
}

/// Build a system prompt fragment describing configured messaging channels.
///
/// Returns `None` if no channels are configured.
pub(crate) fn build_channel_context(config: &crate::config::AppConfig) -> Option<String> {
    let claw = config.claw.as_ref()?;
    if !claw.enabled || claw.channels.is_empty() {
        return None;
    }
    let mut lines = vec!["<channels>".to_string()];
    lines.push(
        "The following messaging channels are already configured and active. \
        Do NOT re-onboard or reconfigure them. Use the existing channel skills to send messages."
            .to_string(),
    );
    for ch in &claw.channels {
        match ch {
            crate::channels::config::ChannelConfig::Lark(lark) => {
                lines.push(format!(
                    "- Feishu/Lark: app_id={}, api_base={}, instance_id={}",
                    lark.app_id, lark.api_base, lark.instance_id
                ));
            }
        }
    }
    lines.push("</channels>".to_string());
    Some(lines.join("\n"))
}

/// Submit the chat panel input to the LLM.
pub(super) fn submit_chat(state: &mut AppState, sid: SessionId, user_text: &str, cell_id: Option<u32>) {
    if user_text.is_empty() {
        return;
    }

    // ── Budget enforcement ────────────────────────────────────────────────
    if let Some(ref budget_arc) = state.budget_tracker
        && let Ok(tracker) = budget_arc.lock()
    {
        let ctx_used = state
            .session_chats
            .get(&sid)
            .map(|c| c.history.total_tokens())
            .unwrap_or(0);
        let ctx_limit = state
            .llm_client
            .as_ref()
            .map_or(128_000, |c| c.max_context_tokens());
        let check = tracker.pre_check(sid, ctx_used, ctx_limit);
        if !check.is_allowed() {
            // Show denial in chat.
            if let Some(chat) = state.session_chats.get_mut(&sid) {
                let reason = check
                    .denial_reason()
                    .unwrap_or_else(|| "Budget limit reached.".into());
                let msg = crate::ai::context::ChatMessage::new(
                    crate::ai::context::MessageRole::Assistant,
                    format!("⚠️ {}", reason),
                    crate::ai::context::MessageImportance::Ephemeral,
                    0,
                );
                chat.history.push(msg);
                chat.is_streaming = false;
            }
            // Log the budget denial to audit log.
            if let Some(ref db) = state.db_store {
                let _ = db.insert_audit_log(
                    sid,
                    "budget_denied",
                    &check.denial_reason().unwrap_or_default(),
                    "denied",
                    None,
                );
            }
            return;
        }
    }

    // ── Parse @agent-name prefix ──────────────────────────────────────────
    let (agent_manifest, actual_text) = parse_agent_prefix(user_text, &state.agent_registry);
    let user_text = &actual_text;

    // Ensure a per-session chat state exists for this session.
    if !state.session_chats.contains_key(&sid) {
        let (ctx, res) = llm_context_limits(state);
        let mut chat = crate::ui::chat::SessionChat::new(ctx, res);
        if let Some(sp) = state
            .llm_client
            .as_ref()
            .and_then(|c| c.system_prompt().map(String::from))
        {
            // Append channel context so the LLM knows about configured integrations.
            let full_prompt = if let Some(ctx) = build_channel_context(&state.config) {
                format!("{}\n\n{}", sp, ctx)
            } else {
                sp
            };
            let model = llm_model_name(state);
            chat.set_system_prompt(&full_prompt, &state.token_counter, &model);
        }
        state.session_chats.insert(sid, chat);
    }

    // If an agent is invoked, inject agent system prompt and register skills.
    if let Some(ref manifest) = agent_manifest {
        // Inject agent system prompt (prepended to existing system prompt).
        if let Some(ref sp) = manifest.system_prompt {
            let base_prompt = state
                .llm_client
                .as_ref()
                .and_then(|c| c.system_prompt().map(String::from));
            let model = llm_model_name(state);
            let chat = state.session_chats.get_mut(&sid).unwrap();
            let combined = if let Some(ref base) = base_prompt {
                format!("{}\n\n{}", sp.template, base)
            } else {
                sp.template.clone()
            };
            chat.set_system_prompt(&combined, &state.token_counter, &model);
        }

        // Register agent skills with namespace if not already registered.
        for skill in &manifest.skills {
            let namespaced = format!("{}.{}", manifest.agent.name, skill.name);
            if state.skill_registry.get(&namespaced).is_none() {
                let skill_def = crate::ai::skills::Skill {
                    name: namespaced.clone(),
                    description: skill.description.clone(),
                    parameters_schema: skill.parameters.clone(),
                    category: "internal".into(),
                };

                // Try to find a matching built-in skill handler.
                // Convention: agent skill name without agent prefix may match
                // a built-in (e.g. "read_file" → "cronymax.fs.read_file" or "read_file").
                let builtin_names = [
                    skill.name.clone(),
                    format!("cronymax.fs.{}", skill.name),
                    format!("cronymax.general.{}", skill.name),
                    format!("cronymax.terminal.{}", skill.name),
                ];
                let handler: crate::ai::skills::SkillHandler =
                    if let Some(existing) = builtin_names.iter().find_map(|n| {
                        state.skill_registry.get(n).map(|(_, h)| h.clone())
                    }) {
                        // Delegate to the existing built-in handler.
                        existing
                    } else {
                        // No matching built-in — run as a pass-through that returns
                        // the skill parameters as context for the LLM to incorporate.
                        let skill_name = skill.name.clone();
                        let skill_desc = skill.description.clone();
                        std::sync::Arc::new(move |args: serde_json::Value| {
                            let name = skill_name.clone();
                            let desc = skill_desc.clone();
                            Box::pin(async move {
                                Ok(serde_json::json!({
                                    "status": "executed",
                                    "skill": name,
                                    "description": desc,
                                    "parameters": args,
                                    "note": "Agent skill executed. The parameters were passed through for LLM processing."
                                }))
                            })
                        })
                    };
                state.skill_registry.register(skill_def, handler);
            }
        }
    }

    let model = llm_model_name(state);

    // ── Terminal context injection ────────────────────────────────────────
    // If the prompt has terminal_context enabled, capture the last 50 lines
    // of the active terminal and inject as an ephemeral system message.
    let terminal_ctx = if state
        .ui_state
        .prompt_editors
        .get(&sid)
        .is_some_and(|p| p.terminal_context)
    {
        if let Some(session) = state.sessions.get(&sid) {
            let history = session.state.history_size() as i32;
            let screen = session.state.viewport_rows() as i32;
            let total = history + screen;
            let start = (total - 50).max(0);
            let text = session.state.capture_text(start, total);
            if !text.trim().is_empty() {
                Some(text)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Submit user message to per-session chat and get API messages.
    let chat = state.session_chats.get_mut(&sid).unwrap();

    // Reset tool round counter on each new user submission.
    chat.tool_rounds = 0;

    // Inject terminal context before user message if available.
    if let Some(ref ctx_text) = terminal_ctx {
        let ctx_msg = crate::ai::context::ChatMessage::new(
            crate::ai::context::MessageRole::System,
            format!(
                "<terminal_context>\nThe following is the recent terminal output for this session:\n{}\n</terminal_context>",
                ctx_text
            ),
            crate::ai::context::MessageImportance::Ephemeral,
            0,
        );
        chat.history.push(ctx_msg);
    }

    let api_messages = chat.submit_user_message(user_text, &state.token_counter, &model, cell_id);
    let llm_session_id = chat.llm_session_id;

    // Map LLM session ID → terminal session ID for event routing.
    state.llm_session_map.insert(llm_session_id, sid);

    // Start streaming.
    if let Some(ref client) = state.llm_client {
        let tools = Some(state.skill_registry.to_openai_tools());
        let handle = client.stream_chat(
            api_messages,
            tools,
            state.proxy.clone(),
            llm_session_id,
            &state.runtime,
        );
        if let Some(chat) = state.session_chats.get_mut(&sid) {
            chat.active_stream = Some(handle);
        }
        log::info!("Chat submitted for session {}, streaming...", sid);
    } else {
        log::warn!("No LLM client configured — cannot submit chat");
        if let Some(chat) = state.session_chats.get_mut(&sid) {
            chat.is_streaming = false;
            let err_msg = crate::ai::context::ChatMessage::new(
                crate::ai::context::MessageRole::Assistant,
                t("error.no_llm").to_string(),
                crate::ai::context::MessageImportance::Ephemeral,
                0,
            );
            chat.add_message(err_msg);
        }
    }

    state.scheduler.mark_dirty();
}
