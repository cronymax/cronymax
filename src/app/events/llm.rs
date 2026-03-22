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
                // Also update the streaming BlockMode::Stream response.
                if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                    for block in prompt_editor.blocks.iter_mut().rev() {
                        if let BlockMode::Stream {
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
                    if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                        for block in prompt_editor.blocks.iter_mut().rev() {
                            if let BlockMode::Stream {
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
                                    tool_calls_log.push(crate::ui::block::ToolCallEntry {
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
                    if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                        for block in prompt_editor.blocks.iter_mut().rev() {
                            if let BlockMode::Stream {
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

            // Handle tool calls if any.
            if !tool_calls.is_empty() {
                for tool_call in &tool_calls {
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
                let record = crate::app::session_persist::chat_to_record(pid, chat);
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

            state.scheduler.mark_dirty();
        }
        AppEvent::LlmError { session_id, error } => {
            log::error!("LlmError[{}]: {}", session_id, error);
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
                // Mark the streaming BlockMode::Stream as error.
                if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                    for block in prompt_editor.blocks.iter_mut().rev() {
                        if let BlockMode::Stream {
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
                msg.tool_call_id = Some(tool_call_id);
                chat.history.push(msg.clone());
                chat.add_message(msg);

                // Re-call stream_chat with updated history including tool results.
                if let Some(ref client) = state.llm_client {
                    let messages = chat.history.for_api();
                    let tools = Some(state.skill_registry.to_openai_tools());
                    chat.is_streaming = true;
                    chat.llm_session_id += 1;
                    let llm_sid = chat.llm_session_id;
                    state.llm_session_map.insert(llm_sid, terminal_sid);
                    let handle = client.stream_chat(
                        messages,
                        tools,
                        state.proxy.clone(),
                        llm_sid,
                        &state.runtime,
                    );
                    chat.active_stream = Some(handle);
                }

                // Clear tool_status and response in the block so
                // follow-up tokens stream into a fresh response.
                // Also mark completed tool_calls_log entries.
                if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&terminal_sid) {
                    for block in prompt_editor.blocks.iter_mut().rev() {
                        if let BlockMode::Stream {
                            is_streaming,
                            response,
                            tool_status,
                            tool_calls_log,
                            ..
                        } = block
                            && *is_streaming
                        {
                            *tool_status = None;
                            response.clear();
                            // Mark the last in-progress tool call as completed
                            // and store its result.
                            for entry in tool_calls_log.iter_mut().rev() {
                                if entry.in_progress {
                                    entry.in_progress = false;
                                    // Store truncated result for display.
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
