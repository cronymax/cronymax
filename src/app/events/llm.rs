//! LLM and tool event handlers

use crate::app::*;

pub(in crate::app) fn handle_llm_event(
    state: &mut AppState,
    event: AppEvent,
    _event_loop: &ActiveEventLoop,
) {
    match event {
        AppEvent::LlmToken { session_id, token } => {
            log::trace!("LlmToken[{}]: {}", session_id, &token);
            // Route token to the correct per-session chat + Block.
            if let Some(&terminal_sid) = state.llm_session_map.get(&session_id) {
                if let Some(chat) = state.session_chats.get_mut(&terminal_sid) {
                    chat.append_token(&token);
                }
                // Also update the streaming Block::Stream response.
                if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                    for block in prompt_editor.blocks.iter_mut().rev() {
                        if let Block::Stream {
                            is_streaming: true,
                            response,
                            ..
                        } = block
                        {
                            response.push_str(&token);
                            break;
                        }
                    }
                }
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::LlmDone {
            session_id,
            full_response,
            usage,
            tool_calls,
        } => {
            log::info!(
                "LlmDone[{}]: {} chars, usage={:?}, {} tool_calls",
                session_id,
                full_response.len(),
                usage,
                tool_calls.len()
            );

            // ── Channel reply routing ────────────────────────────────
            // Session IDs >= 900_000 are reserved for channel messages.
            if let Some(reply_target) = state.pending_channel_replies.remove(&session_id) {
                log::info!(
                    "Routing LLM response to channel {} (chat={}): {} chars",
                    reply_target.channel_id,
                    reply_target.chat_id,
                    full_response.len(),
                );
                let _ = state.proxy.send_event(AppEvent::ChannelSendReply {
                    target: reply_target,
                    content: full_response.clone(),
                });
                state.scheduler.mark_dirty();
                // Skip normal per-session chat routing for channel messages.
                return;
            }

            // Route to the correct per-session chat.
            if let Some(&terminal_sid) = state.llm_session_map.get(&session_id) {
                let model = llm_model_name(state);

                // Determine the cell_id of the currently-streaming block
                // so the assistant message can be linked for thread branching.
                let streaming_cell_id =
                    state
                        .ui_state
                        .prompt_editors
                        .get(&terminal_sid)
                        .and_then(|pe| {
                            pe.blocks.iter().rev().find_map(|b| {
                                if let Block::Stream {
                                    id, is_streaming, ..
                                } = b
                                {
                                    if *is_streaming { Some(*id) } else { None }
                                } else {
                                    None
                                }
                            })
                        });

                if let Some(chat) = state.session_chats.get_mut(&terminal_sid) {
                    // Update token usage display.
                    if let Some(ref u) = usage {
                        chat.tokens_used = u.total_tokens;
                    }

                    // Create assistant message (include tool_calls so
                    // re-sent history satisfies the OpenAI API contract).
                    let tc = state.token_counter.count(&full_response, &model) as u32;
                    let mut msg = crate::ai::context::ChatMessage::new(
                        crate::ai::context::MessageRole::Assistant,
                        full_response.clone(),
                        crate::ai::context::MessageImportance::Normal,
                        tc,
                    );
                    msg.tool_calls = tool_calls.clone();
                    msg.cell_id = streaming_cell_id;
                    chat.history.push(msg.clone());
                    chat.finalize_streaming(msg);

                    // Record budget usage for this turn.
                    if let Some(ref budget_arc) = state.budget_tracker
                        && let Ok(tracker) = budget_arc.lock()
                    {
                        let _ = tracker.record_usage(terminal_sid, tc as i64);
                    }

                    // Check if compaction needed (>80% of context used).
                    let total = chat.history.total_tokens();
                    let limit = chat.history.max_context_tokens;
                    if total > limit * 80 / 100 {
                        let _ = state.proxy.send_event(AppEvent::CompactionNeeded {
                            session_id,
                            used_tokens: total as u32,
                            limit_tokens: limit as u32,
                        });
                    }
                }
                // When tool_calls are present, keep the block streaming and
                // show a tool invocation status instead of finalizing.
                if !tool_calls.is_empty() {
                    // Build a human-readable status of which tools are being called,
                    // including key arguments for better user visibility.
                    let status_parts: Vec<String> = tool_calls
                        .iter()
                        .map(|tc| {
                            let args: serde_json::Value = serde_json::from_str(&tc.arguments)
                                .unwrap_or(serde_json::Value::Null);
                            match tc.function_name.as_str() {
                                "cronymax.terminal.execute" => {
                                    let cmd = args["command"].as_str().unwrap_or("...");
                                    let short = if cmd.len() > 60 { &cmd[..60] } else { cmd };
                                    format!("terminal_execute_command: {}", short)
                                }
                                "cronymax.webview.open" => {
                                    let url = args["url"].as_str().unwrap_or("...");
                                    format!("open_webview: {}", url)
                                }
                                "cronymax.webview.navigate" => {
                                    let url = args["url"].as_str().unwrap_or("...");
                                    format!("navigate_webview: {}", url)
                                }
                                "cronymax.browser.extract_text" => {
                                    let sel = args["selector"].as_str().unwrap_or("body");
                                    format!("browser_extract_text({})", sel)
                                }
                                other => other.to_string(),
                            }
                        })
                        .collect();
                    let status_text = status_parts.join(", ");
                    if let Some(prompt_editor) =
                        state.ui_state.prompt_editors.get_mut(&terminal_sid)
                    {
                        for block in prompt_editor.blocks.iter_mut().rev() {
                            if let Block::Stream {
                                is_streaming,
                                tool_status,
                                tool_calls_log,
                                ..
                            } = block
                                && *is_streaming
                            {
                                *tool_status = Some(status_text.clone());
                                // Add tool call log entries for each invoked tool.
                                for tc in &tool_calls {
                                    let args: serde_json::Value =
                                        serde_json::from_str(&tc.arguments)
                                            .unwrap_or(serde_json::Value::Null);
                                    let summary = match tc.function_name.as_str() {
                                        "cronymax.terminal.execute" => {
                                            let cmd = args["command"].as_str().unwrap_or("...");
                                            format!(
                                                "`{}`",
                                                if cmd.len() > 80 { &cmd[..80] } else { cmd }
                                            )
                                        }
                                        "cronymax.webview.open" | "cronymax.webview.navigate" => {
                                            args["url"].as_str().unwrap_or("...").to_string()
                                        }
                                        "cronymax.browser.extract_text" => {
                                            format!(
                                                "selector: {}",
                                                args["selector"].as_str().unwrap_or("body")
                                            )
                                        }
                                        _ => tc.arguments.chars().take(80).collect(),
                                    };
                                    tool_calls_log.push(crate::ui::blocks::ToolCallEntry {
                                        name: tc.function_name.clone(),
                                        summary,
                                        result: None,
                                        expanded: false,
                                        in_progress: true,
                                    });
                                }
                                break;
                            }
                        }
                    }
                } else {
                    // No tool calls — this is a final response. Finalize the block.
                    if let Some(prompt_editor) =
                        state.ui_state.prompt_editors.get_mut(&terminal_sid)
                    {
                        for block in prompt_editor.blocks.iter_mut().rev() {
                            if let Block::Stream {
                                is_streaming,
                                response,
                                tool_status,
                                ..
                            } = block
                                && *is_streaming
                            {
                                *is_streaming = false;
                                *tool_status = None;
                                *response = full_response.clone();
                                break;
                            }
                        }
                    }
                }
            }

            // Handle tool calls if any — apply SubagentLimit middleware
            // then register pending calls for parallel collection.
            if !tool_calls.is_empty() {
                // Run after_llm middleware (e.g., SubagentLimitMiddleware
                // truncates excess concurrent tool calls).
                let effective_tool_calls = if let Some(&terminal_sid) =
                    state.llm_session_map.get(&session_id)
                    && let Some(chat) = state.session_chats.get(&terminal_sid)
                {
                    let mut mw_ctx = crate::ai::middleware::MiddlewareContext::new(
                        chat.tool_rounds,
                        chat.max_tool_rounds,
                        chat.history.total_tokens(),
                        chat.history.max_context_tokens,
                    );
                    let outcome = chat.middleware_chain.run_after_llm(
                        &full_response,
                        &tool_calls,
                        &mut mw_ctx,
                    );
                    outcome
                        .override_tool_calls
                        .unwrap_or_else(|| tool_calls.clone())
                } else {
                    tool_calls.clone()
                };

                // Register all tool calls as pending — the LLM will only
                // be re-invoked once ALL results have been collected.
                if let Some(&terminal_sid) = state.llm_session_map.get(&session_id)
                    && let Some(chat) = state.session_chats.get_mut(&terminal_sid)
                {
                    chat.pending_tool_calls.clear();
                    for tc in &effective_tool_calls {
                        chat.pending_tool_calls.insert(tc.id.clone());
                    }
                }

                for tool_call in &effective_tool_calls {
                    if let Some((_, handler)) = state.skill_registry.get(&tool_call.function_name) {
                        let args: serde_json::Value = serde_json::from_str(&tool_call.arguments)
                            .unwrap_or(serde_json::Value::Null);
                        let handler = handler.clone();
                        let proxy = state.proxy.clone();
                        let sid = session_id;
                        let tc_id = tool_call.id.clone();
                        state.runtime.spawn(async move {
                            match handler(args).await {
                                Ok(result) => {
                                    let _ = proxy.send_event(AppEvent::ToolResult {
                                        session_id: sid,
                                        tool_call_id: tc_id,
                                        result: result.to_string(),
                                    });
                                }
                                Err(e) => {
                                    let _ = proxy.send_event(AppEvent::ToolResult {
                                        session_id: sid,
                                        tool_call_id: tc_id,
                                        result: format!("{{\"error\": \"{}\"}}", e),
                                    });
                                }
                            }
                        });
                    } else {
                        // Unknown tool — immediately send error result so
                        // the pending set is drained properly.
                        let _ = state.proxy.send_event(AppEvent::ToolResult {
                            session_id,
                            tool_call_id: tool_call.id.clone(),
                            result: format!(
                                "{{\"error\": \"Unknown tool: {}\"}}",
                                tool_call.function_name
                            ),
                        });
                    }
                }
            }

            // ── Per-message auto-save (T034) ────────────────────────────
            // After the assistant response is finalized, persist the chat
            // session to disk asynchronously so history survives crashes.
            if tool_calls.is_empty()
                && let Some(&terminal_sid) = state.llm_session_map.get(&session_id)
                && let Some(chat) = state.session_chats.get(&terminal_sid)
                && let Some(ref pid) = chat.persistent_id
            {
                let record =
                    crate::app::session_persist::chat_to_record(pid, chat, &state.session_chats);
                let mgr = state.profile_manager.lock().unwrap();
                let profile_dir = mgr
                    .active()
                    .map(|p| mgr.profile_dir(&p.id))
                    .unwrap_or_else(|| mgr.profile_dir("default"));
                drop(mgr);
                let uuid = pid.clone();
                state.runtime.spawn(async move {
                    if let Err(e) =
                        crate::app::session_persist::save_session_file(&uuid, &record, &profile_dir)
                    {
                        log::warn!("Auto-save session {}: {}", uuid, e);
                    }
                });
            }

            // ── Always-On Memory Agent (background extraction) ──────────
            // After the final response (no tool calls), check if enough
            // new messages have accumulated to trigger memory extraction.
            if tool_calls.is_empty()
                && let Some(&terminal_sid) = state.llm_session_map.get(&session_id)
                && let Some(chat) = state.session_chats.get(&terminal_sid)
            {
                let memory_store = state.memory_store.clone();
                let memory_agent_config = state.memory_agent.config().clone();
                let conversation: Vec<crate::ai::context::ChatMessage> = chat.history.for_api();
                let openai_client = state.llm_client.as_ref().and_then(|c| c.openai_client());

                // Debounce: 2 new messages per turn (user + assistant).
                let should_extract = {
                    let agent = &state.memory_agent;
                    let rt = state.runtime.clone();
                    rt.block_on(agent.notify_new_messages(2))
                };

                if should_extract
                    && memory_agent_config.enabled
                    && let Some(oai_client) = openai_client
                {
                    let agent =
                        crate::ai::memory_agent::MemoryAgent::new(memory_agent_config.clone());
                    state.runtime.spawn(async move {
                        let existing_entries = {
                            let store = memory_store.lock().unwrap();
                            store.entries.clone()
                        };
                        let extraction_messages =
                            agent.build_extraction_messages(&conversation, &existing_entries);

                        // Use the non-streaming LLM backend to call the cheap model.
                        let backend = crate::ai::agent_loop::NonStreamingLlmBackend {
                            client: oai_client,
                            model: agent.config().model.clone(),
                        };
                        match crate::ai::agent_loop::LlmBackend::complete(
                            &backend,
                            &extraction_messages,
                            None,
                        )
                        .await
                        {
                            Ok(result) => {
                                let facts =
                                    crate::ai::memory_agent::MemoryAgent::parse_extraction_response(
                                        &result.response,
                                    );
                                if !facts.is_empty() {
                                    let entries =
                                        crate::ai::memory_agent::MemoryAgent::facts_to_entries(
                                            &facts,
                                        );
                                    let mut store = memory_store.lock().unwrap();
                                    for entry in entries {
                                        store.insert(entry);
                                    }
                                    log::info!(
                                        "[MemoryAgent] Extracted {} facts from conversation",
                                        facts.len()
                                    );
                                }
                            }
                            Err(e) => {
                                log::warn!("[MemoryAgent] Background extraction failed: {}", e);
                            }
                        }
                        agent.reset_counter().await;
                    });
                }
            }

            state.scheduler.mark_dirty();
        }
        AppEvent::LlmError { session_id, error } => {
            log::error!("LlmError[{}]: {}", session_id, error);
            state.ui_state.notifications.error("LLM Error", &error);
            // Route error to the correct per-session chat + Block.
            if let Some(&terminal_sid) = state.llm_session_map.get(&session_id) {
                if let Some(chat) = state.session_chats.get_mut(&terminal_sid) {
                    chat.is_streaming = false;
                    let msg = crate::ai::context::ChatMessage::new(
                        crate::ai::context::MessageRole::Assistant,
                        t_fmt("error.llm_fmt", &error),
                        crate::ai::context::MessageImportance::Ephemeral,
                        0,
                    );
                    chat.add_message(msg);
                }
                // Mark the streaming Block::Stream as error.
                if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                    for block in prompt_editor.blocks.iter_mut().rev() {
                        if let Block::Stream {
                            is_streaming,
                            response,
                            tool_status,
                            ..
                        } = block
                            && *is_streaming
                        {
                            *is_streaming = false;
                            *tool_status = None;
                            *response = t_fmt("error.llm_fmt", &error);
                            break;
                        }
                    }
                }
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::CompactionNeeded {
            session_id,
            used_tokens,
            limit_tokens,
        } => {
            log::warn!(
                "CompactionNeeded[{}]: {}/{} tokens",
                session_id,
                used_tokens,
                limit_tokens
            );
            // Mark compaction needed in the per-session chat.
            // (Future: auto-compact or show a UI indicator per-cell.)
            state.scheduler.mark_dirty();
        }
        AppEvent::ToolResult {
            session_id,
            tool_call_id,
            result,
        } => {
            // Truncate oversized tool results to avoid 413 Payload Too Large
            // from the LLM API. 16 KiB of text leaves plenty of room in the
            // context window while staying well under HTTP body limits.
            const MAX_TOOL_RESULT_BYTES: usize = 16_384;
            let result = if result.len() > MAX_TOOL_RESULT_BYTES {
                let truncated = &result[..result.floor_char_boundary(MAX_TOOL_RESULT_BYTES)];
                format!("{}\n[…truncated, {} bytes total]", truncated, result.len())
            } else {
                result
            };

            log::info!(
                "ToolResult[{}]: call={}, result_len={}",
                session_id,
                tool_call_id,
                result.len()
            );

            // Audit log the tool execution.
            if let Some(ref db) = state.db_store {
                let terminal_sid = state.llm_session_map.get(&session_id).copied().unwrap_or(0);
                let _ = db.insert_audit_log(terminal_sid, "tool_call", &tool_call_id, "ok", None);
            }

            // Route tool result to the correct per-session chat.
            let model = llm_model_name(state);
            if let Some(&terminal_sid) = state.llm_session_map.get(&session_id)
                && let Some(chat) = state.session_chats.get_mut(&terminal_sid)
            {
                let result_for_log = result.clone();
                let tc = state.token_counter.count(&result, &model) as u32;
                let mut msg = crate::ai::context::ChatMessage::new(
                    crate::ai::context::MessageRole::Tool,
                    result,
                    crate::ai::context::MessageImportance::Normal,
                    tc,
                );
                msg.tool_call_id = Some(tool_call_id.clone());
                chat.history.push(msg.clone());
                chat.add_message(msg);

                // Remove this tool call from the pending set.
                chat.pending_tool_calls.remove(&tool_call_id);
                let all_collected = chat.pending_tool_calls.is_empty();

                // Mark completed tool_calls_log entry in the streaming block.
                if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                    for block in prompt_editor.blocks.iter_mut().rev() {
                        if let Block::Stream {
                            is_streaming,
                            tool_calls_log,
                            ..
                        } = block
                            && *is_streaming
                        {
                            // Mark the matching tool call as completed.
                            for entry in tool_calls_log.iter_mut() {
                                if entry.in_progress {
                                    entry.in_progress = false;
                                    let result_str = &result_for_log;
                                    entry.result = Some(if result_str.len() > 4000 {
                                        format!("{}…", &result_str[..4000])
                                    } else {
                                        result_str.clone()
                                    });
                                    break;
                                }
                            }
                            break;
                        }
                    }
                }

                // Only re-invoke the LLM once ALL parallel tool results
                // have been collected (DeerFlow-inspired batch pattern).
                if !all_collected {
                    log::info!(
                        "ToolResult[{}]: {} tool call(s) still pending, waiting...",
                        session_id,
                        chat.pending_tool_calls.len()
                    );
                    state.scheduler.mark_dirty();
                    return;
                }

                // All tool results collected — increment tool round and
                // run before_llm middleware chain before re-invocation.
                chat.tool_rounds += 1;

                // Build middleware context and run before_llm chain
                // (handles: dangling tool calls, context summarization,
                // tool round guard).
                let mut mw_ctx = crate::ai::middleware::MiddlewareContext::new(
                    chat.tool_rounds,
                    chat.max_tool_rounds,
                    chat.history.total_tokens(),
                    chat.history.max_context_tokens,
                );
                let mut api_messages = chat.history.for_api();
                let should_proceed = chat
                    .middleware_chain
                    .run_before_llm(&mut api_messages, &mut mw_ctx);

                if !should_proceed {
                    log::info!(
                        "Agent loop for session {} stopped by middleware: {}",
                        terminal_sid,
                        mw_ctx.abort_reason.as_deref().unwrap_or("unknown")
                    );
                    chat.is_streaming = false;
                    let stop_msg = crate::ai::context::ChatMessage::new(
                        crate::ai::context::MessageRole::Assistant,
                        format!(
                            "⚠️ {}",
                            mw_ctx
                                .abort_reason
                                .as_deref()
                                .unwrap_or("Agent loop stopped by middleware.")
                        ),
                        crate::ai::context::MessageImportance::Ephemeral,
                        0,
                    );
                    chat.add_message(stop_msg);
                    chat.tool_rounds = 0;
                    state.scheduler.mark_dirty();
                    return;
                }

                // Re-call stream_chat with updated history including all tool results.
                if let Some(ref client) = state.llm_client {
                    let tools = Some(state.skill_registry.to_openai_tools());
                    chat.is_streaming = true;
                    chat.llm_session_id += 1;
                    let llm_sid = chat.llm_session_id;
                    state.llm_session_map.insert(llm_sid, terminal_sid);
                    let handle = client.stream_chat(
                        api_messages,
                        tools,
                        state.proxy.clone(),
                        llm_sid,
                        &state.runtime,
                    );
                    chat.active_stream = Some(handle);
                }

                // Clear tool_status and response in the block for the next LLM turn.
                if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                    for block in prompt_editor.blocks.iter_mut().rev() {
                        if let Block::Stream {
                            is_streaming,
                            response,
                            tool_status,
                            ..
                        } = block
                            && *is_streaming
                        {
                            *tool_status = None;
                            response.clear();
                            break;
                        }
                    }
                }
            }

            state.scheduler.mark_dirty();
        }
        AppEvent::SkillUiAction {
            session_id,
            tool_call_id,
            action,
            result,
        } => {
            log::info!(
                "SkillUiAction[{}]: call={}, action={:?}",
                session_id,
                tool_call_id,
                action
            );
            // Execute the UI action on the main thread.
            state.dispatch_ui_action(action, _event_loop);
            // Feed the result back to the agentic loop as a ToolResult.
            let _ = state.proxy.send_event(AppEvent::ToolResult {
                session_id,
                tool_call_id,
                result,
            });
            state.scheduler.mark_dirty();
        }
        _ => {}
    }
}
