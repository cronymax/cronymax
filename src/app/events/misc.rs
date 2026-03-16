//! Miscellaneous app event handlers

use crate::app::*;

pub(in crate::app) fn handle_misc_event(
    state: &mut AppState,
    event: AppEvent,
    _event_loop: &ActiveEventLoop,
) {
    match event {
        AppEvent::ScheduledTaskStarted { task_id, task_name } => {
            log::info!("Scheduled task started: {} ({})", task_name, task_id);
            // Push info block to the active session's chat.
            if let Some(sid) = state.ui_state.focused_terminal_session {
                push_info_block(
                    state,
                    sid,
                    &format!("⏱ Scheduled task started: {}", task_name),
                );
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::ScheduledTaskFire {
            task_id,
            task_name,
            action_type,
            action_value,
        } => {
            log::info!(
                "ScheduledTaskFire: {} ({}) — {} '{}'",
                task_name,
                task_id,
                action_type,
                action_value,
            );
            // Execute the task action on the main thread.
            match action_type.as_str() {
                "prompt" => {
                    // Find the active session and submit the prompt as a chat message.
                    if let Some(sid) = state.ui_state.focused_terminal_session {
                        if state.llm_client.is_some() {
                            // Refresh the session's system prompt with current channel
                            // context so the LLM knows about configured integrations.
                            let base_sp = state
                                .llm_client
                                .as_ref()
                                .and_then(|c| c.system_prompt().map(String::from));
                            let model = llm_model_name(state);
                            let channel_ctx = build_channel_context(&state.config);
                            if let (Some(chat), Some(sp)) =
                                (state.session_chats.get_mut(&sid), base_sp)
                            {
                                let full_prompt = if let Some(ctx) = channel_ctx {
                                    format!("{}\n\n{}", sp, ctx)
                                } else {
                                    sp
                                };
                                chat.set_system_prompt(&full_prompt, &state.token_counter, &model);
                            }
                            push_info_block(
                                state,
                                sid,
                                &format!(
                                    "⏱ Running scheduled task: {} — {}",
                                    task_name, action_value
                                ),
                            );
                            submit_chat(state, sid, &action_value);
                            push_info_block(
                                state,
                                sid,
                                &format!(
                                    "✓ Scheduled task dispatched: {} — prompt submitted to LLM",
                                    task_name
                                ),
                            );
                        } else {
                            push_info_block(
                                state,
                                sid,
                                &format!(
                                    "✗ Scheduled task failed: {} — no LLM provider configured",
                                    task_name
                                ),
                            );
                            log::error!(
                                "ScheduledTaskFire: no LLM client for prompt task {}",
                                task_id
                            );
                        }
                    } else {
                        log::warn!(
                            "ScheduledTaskFire: no focused session for prompt task {}",
                            task_id
                        );
                    }
                }
                _ => {
                    log::warn!(
                        "ScheduledTaskFire: unexpected action_type '{}' for task {}",
                        action_type,
                        task_id
                    );
                }
            }
            // Reload the task store to reflect run_once auto-disable.
            let _ = state.task_store.load();
            state.scheduler.mark_dirty();
        }
        AppEvent::ScheduledTaskCompleted {
            task_id,
            task_name,
            status,
            duration_ms,
            output,
        } => {
            log::info!(
                "Scheduled task completed: {} ({}) — {} in {}ms",
                task_name,
                task_id,
                status,
                duration_ms,
            );
            // Push completion info to the active session's chat.
            if let Some(sid) = state.ui_state.focused_terminal_session {
                let icon = if status == "success" { "✓" } else { "✗" };
                let mut msg = format!(
                    "{} Scheduled task completed: {} — {} ({}ms)",
                    icon, task_name, status, duration_ms
                );
                // Show truncated command output if present.
                if !output.is_empty() {
                    let truncated = if output.len() > 500 {
                        format!("{}…", &output[..500])
                    } else {
                        output.clone()
                    };
                    msg.push_str(&format!("\n\n**Output:**\n```\n{}\n```", truncated));
                }
                push_info_block(state, sid, &msg);
            }
            // Refresh history cache for UI.
            let runtime = crate::ai::scheduler::SchedulerRuntime::new(
                crate::ai::scheduler::ScheduledTaskStore::new(),
            );
            state.scheduler_history_cache = runtime.load_all_history(100);
            // Reload task store to reflect run_once auto-disable.
            let _ = state.task_store.load();
            state.scheduler.mark_dirty();
        }
        AppEvent::ModelsLoaded { models } => {
            log::info!("ModelsLoaded: received {} models from API", models.len());
            for pe in state.prompt_editors.values_mut() {
                pe.model_items = models.clone();
                pe.selected_model_idx = 0;
            }
            // Also populate available models for the profiles settings UI.
            // Use provider_name() for the grouping key so it matches
            // the display name used in the Profiles provider ComboBox.
            state.profiles_ui_state.available_models = models
                .iter()
                .map(|m| (m.provider_name().to_string(), m.model.clone()))
                .collect();
            state.scheduler.mark_dirty();
        }
        AppEvent::InjectScript {
            webview_id,
            script,
            request_id,
        } => {
            log::info!(
                "InjectScript[{}]: webview={}, script_len={}",
                request_id,
                webview_id,
                script.len()
            );
            // Find the webview by ID; 0 means "active webview".
            let target_idx = if webview_id != 0 {
                state.webview_tabs.iter().position(|wt| wt.id == webview_id)
            } else {
                if state.webview_tabs.is_empty() {
                    None
                } else {
                    Some(state.active_webview)
                }
            };
            let found = if let Some(idx) = target_idx {
                if let Some(tab) = state.webview_tabs.get(idx) {
                    let _ = tab.manager.webview.evaluate_script(&script);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !found {
                // Send error back via pending results.
                log::warn!("InjectScript: webview {} not found", webview_id);
                if let Ok(mut map) = state.pending_results.lock()
                    && let Some(sender) = map.remove(&request_id)
                {
                    let _ = sender.send(serde_json::json!({
                        "error": format!("Webview {} not found", webview_id)
                    }));
                }
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::TerminalExec {
            terminal_id,
            command,
            marker,
            timeout_ms,
        } => {
            log::info!(
                "TerminalExec[{}]: cmd_len={}, marker={}",
                terminal_id,
                command.len(),
                &marker[..marker.len().min(40)]
            );

            // ── Sandbox policy enforcement ────────────────────────────
            // Check the command against the active profile's sandbox FS
            // deny rules before allowing execution.
            let sandbox_denied = {
                let mgr = state.profile_manager.lock().unwrap();
                if let Some(profile) = mgr.active() {
                    let policy = profile
                        .sandbox
                        .clone()
                        .unwrap_or_else(crate::profile::sandbox::policy::SandboxPolicy::from_default);
                    policy.check_command(&command).err()
                } else {
                    // No active profile — use default policy.
                    crate::profile::sandbox::policy::SandboxPolicy::from_default()
                        .check_command(&command)
                        .err()
                }
            };

            if let Some(reason) = sandbox_denied {
                log::warn!("TerminalExec blocked by sandbox: {}", reason);
                // Resolve immediately with a sandbox denial error.
                if let Ok(mut map) = state.pending_results.lock()
                    && let Some(sender) = map.remove(&marker)
                {
                    let _ = sender.send(serde_json::json!({
                        "error": format!("Blocked by sandbox policy: {}", reason),
                        "exit_marker_found": false,
                        "output": "",
                        "elapsed_ms": 0
                    }));
                }
                state.scheduler.mark_dirty();
                return;
            }

            // Find the terminal session and write the command + marker.
            let sessions: Vec<(&SessionId, &mut TerminalSession)> =
                state.sessions.iter_mut().collect();
            if let Some((_sid, session)) = sessions.into_iter().nth(terminal_id) {
                // Record the current cursor position as start.
                let start_abs_row = session.state.abs_cursor_row();
                // Write command with marker appended.
                let full_cmd = format!("{} ; echo \"{}\"\n", command, marker);
                session.write_to_pty(full_cmd.as_bytes());
                log::info!(
                    "TerminalExec: start_abs_row={}, cols={}, cmd_len={}",
                    start_abs_row,
                    session.grid_size.cols,
                    full_cmd.len(),
                );
                // Register the pending exec for polling.
                state.pending_terminal_execs.push(PendingTerminalExec {
                    marker: marker.clone(),
                    terminal_id,
                    start_abs_row,
                    started_at: std::time::Instant::now(),
                    timeout_ms,
                    full_cmd: full_cmd.trim_end().to_string(),
                });
            } else {
                // Terminal not found — resolve immediately with error.
                if let Ok(mut map) = state.pending_results.lock()
                    && let Some(sender) = map.remove(&marker)
                {
                    let _ = sender.send(serde_json::json!({
                        "error": format!("Terminal session {} not found", terminal_id),
                        "exit_marker_found": false,
                        "output": "",
                        "elapsed_ms": 0
                    }));
                }
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::ReadTerminalScreen {
            terminal_id,
            start_line,
            end_line,
            max_lines,
            request_id,
        } => {
            log::info!(
                "ReadTerminalScreen[{}]: terminal={}, lines={:?}..{:?}, max={}",
                request_id,
                terminal_id,
                start_line,
                end_line,
                max_lines,
            );
            let sessions: Vec<(&SessionId, &TerminalSession)> = state.sessions.iter().collect();
            let result = if let Some((_sid, session)) = sessions.into_iter().nth(terminal_id) {
                let history = session.state.history_size() as i32;
                let viewport_rows = session.state.viewport_rows() as i32;
                let total_rows = history + viewport_rows;

                // Default: read the current viewport.
                let start = start_line.unwrap_or(history);
                let end = end_line.unwrap_or(total_rows);
                let end = end.min(start + max_lines as i32);

                let text = session.state.capture_text(start, end);
                let lines: Vec<&str> = text.lines().collect();

                let cursor_point = session.state.term.grid().cursor.point;
                serde_json::json!({
                    "lines": lines,
                    "cursor_row": cursor_point.line.0,
                    "cursor_col": cursor_point.column.0,
                    "total_rows": total_rows,
                    "viewport_start": history,
                })
            } else {
                serde_json::json!({
                    "error": format!("Terminal session {} not found", terminal_id)
                })
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(sender) = map.remove(&request_id)
            {
                let _ = sender.send(result);
            }
            state.scheduler.mark_dirty();
        }

        // ── Copilot OAuth events ───────────────────────────────
        AppEvent::CopilotDeviceCode {
            user_code,
            verification_uri,
        } => {
            log::info!(
                "Copilot login: enter code {} at {}",
                user_code,
                verification_uri
            );
            // Open the GitHub device-verification page in the internal webview.
            open_webview(state, &verification_uri, _event_loop);

            // Show the device code in the active chat so the user sees it.
            if let Some(sid) = state.ui_state.focused_terminal_session
                && let Some(chat) = state.session_chats.get_mut(&sid)
            {
                chat.add_message(crate::ai::context::ChatMessage::new(
                    crate::ai::context::MessageRole::Assistant,
                    format!(
                        "**GitHub Copilot Login**\n\n\
                         Enter the code **`{}`** on the GitHub page that just opened.\n\n\
                         Waiting for authorization…",
                        user_code
                    ),
                    crate::ai::context::MessageImportance::Ephemeral,
                    0,
                ));
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::CopilotLoginComplete {
            oauth_token,
            session_token,
            api_base,
        } => {
            log::info!("Copilot login complete — updating client");
            if let Some(ref mut client) = state.llm_client {
                client.complete_copilot_login(oauth_token, &session_token, &api_base);

                // Re-fetch models now that we're authenticated.
                client.fetch_available_models(state.proxy.clone(), &state.runtime);
            }

            // Notify the user in the active chat.
            if let Some(sid) = state.ui_state.focused_terminal_session
                && let Some(chat) = state.session_chats.get_mut(&sid)
            {
                chat.add_message(crate::ai::context::ChatMessage::new(
                    crate::ai::context::MessageRole::Assistant,
                    "**GitHub Copilot**: Login successful! You can now use Copilot models.".into(),
                    crate::ai::context::MessageImportance::Ephemeral,
                    0,
                ));
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::CopilotLoginFailed { error } => {
            log::error!("Copilot login failed: {}", error);
            if let Some(sid) = state.ui_state.focused_terminal_session
                && let Some(chat) = state.session_chats.get_mut(&sid)
            {
                chat.add_message(crate::ai::context::ChatMessage::new(
                    crate::ai::context::MessageRole::Assistant,
                    format!(
                        "**GitHub Copilot**: Login failed — {}\n\nTry `:copilot-login` again.",
                        error
                    ),
                    crate::ai::context::MessageImportance::Ephemeral,
                    0,
                ));
            }
            state.scheduler.mark_dirty();
        }

        // ── Channel subsystem events (Claw mode) ────────────────────
        _ => {}
    }
}
