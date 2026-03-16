//! Channel, onboarding, and scheduler event handlers

use crate::app::*;

pub(in crate::app) fn handle_channel_event(
    state: &mut AppState,
    event: AppEvent,
    _event_loop: &ActiveEventLoop,
) {
    match event {
        AppEvent::ChannelMessageReceived { message } => {
            log::info!(
                "Channel message from {} (channel={}, chat={}): {} chars",
                message.sender_id,
                message.channel_id,
                message.chat_id,
                message.content.len(),
            );

            // Store for display in channel conversation tab.
            let display_msg = crate::channel::ChannelDisplayMessage {
                sender: message
                    .sender_name
                    .clone()
                    .unwrap_or_else(|| message.sender_id.clone()),
                content: message.content.clone(),
                is_outgoing: false,
                timestamp: message.timestamp,
            };
            state
                .channel_messages
                .entry(message.channel_id.clone())
                .or_default()
                .push(display_msg);

            // Route through the 6-stage agent loop pipeline.
            // Prerequisites: LLM client + db_store for memory.
            let openai_client = state.llm_client.as_ref().and_then(|c| c.openai_client());
            let Some(openai_client) = openai_client else {
                log::warn!("Channel message received but no LLM client configured");
                state.scheduler.mark_dirty();
                return;
            };

            // Look up the channel's profile_id from claw config.
            let profile_id = state
                .config
                .claw
                .as_ref()
                .and_then(|c| {
                    c.channels
                        .iter()
                        .map(|ch| match ch {
                            crate::channel::config::ChannelConfig::Lark(cfg) => {
                                cfg.profile_id.clone()
                            }
                        })
                        .next()
                })
                .unwrap_or_else(|| "default".to_string());

            // Get allowed_skills from the bound profile.
            let allowed_skills = {
                let mgr = state.profile_manager.lock().unwrap();
                mgr.profiles
                    .get(&profile_id)
                    .map(|p| p.allowed_skills.clone())
                    .unwrap_or_else(crate::profile::store::default_allowed_skills)
            };

            // Build filtered tool definitions and handlers.
            let tools = state
                .skill_registry
                .to_openai_tools_filtered(&allowed_skills);
            let skill_handlers = state.skill_registry.handlers_filtered(&allowed_skills);

            let model = state
                .llm_client
                .as_ref()
                .map(|c| c.model_name().to_string())
                .unwrap_or_else(|| "gpt-4o".to_string());

            let mut system_prompt = state
                .config
                .ai
                .as_ref()
                .and_then(|a| a.system_prompt.clone())
                .unwrap_or_default();

            // Inject external skill instructions into system prompt.
            if allowed_skills.iter().any(|c| c == "external") {
                for ext in &state.loaded_external_skills {
                    if !ext.instructions.is_empty() {
                        system_prompt.push_str(&format!(
                            "\n\n--- Skill: {} ---\n{}",
                            ext.frontmatter.name, ext.instructions
                        ));
                    }
                }
            }

            let deps = crate::channel::agent_loop::AgentLoopDeps {
                openai_client,
                model,
                system_prompt,
                tools,
                skill_handlers,
            };
            let loop_config = crate::channel::agent_loop::AgentLoopConfig::default();

            // Create ChannelMemoryStore (without embedder for now).
            let memory = crate::channel::memory::ChannelMemoryStore::new(
                std::sync::Arc::new(state.db_store.clone().unwrap_or_else(|| {
                    // Fallback: open a transient in-memory db.
                    crate::ai::db::DbStore::open(&std::path::PathBuf::from(":memory:"))
                        .expect("in-memory db")
                })),
                None, // embedder
            );

            let reply_target = message.reply_target.clone();
            let proxy = state.proxy.clone();

            state.runtime.spawn(async move {
                match crate::channel::agent_loop::process_message(
                    &message,
                    &loop_config,
                    &memory,
                    &deps,
                )
                .await
                {
                    Ok(response) => {
                        let _ = proxy.send_event(AppEvent::ChannelSendReply {
                            target: reply_target,
                            content: response,
                        });
                    }
                    Err(e) => {
                        log::error!("Agent loop error: {}", e);
                        let _ = proxy.send_event(AppEvent::ChannelSendReply {
                            target: reply_target,
                            content: format!("Error: {}", e),
                        });
                    }
                }
            });
            state.scheduler.mark_dirty();
        }

        AppEvent::ChannelSendReply { target, content } => {
            log::info!(
                "Sending channel reply to {} (channel={}, {} chars)",
                target.chat_id,
                target.channel_id,
                content.len(),
            );

            // Store for display in channel conversation tab.
            let display_msg = crate::channel::ChannelDisplayMessage {
                sender: "Bot".to_string(),
                content: content.clone(),
                is_outgoing: true,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
            };
            state
                .channel_messages
                .entry(target.channel_id.clone())
                .or_default()
                .push(display_msg);

            // Send via a fresh LarkChannel instance (stateless REST call).
            // Find the first Lark channel config from the channels Vec.
            let lark_cfg = state
                .config
                .claw
                .as_ref()
                .and_then(|c| {
                    c.channels
                        .iter()
                        .map(|ch| match ch {
                            crate::channel::config::ChannelConfig::Lark(cfg) => cfg.clone(),
                        })
                        .next()
                })
                // Fallback to legacy lark field.
                .or_else(|| state.config.claw.as_ref().and_then(|c| c.lark.clone()));

            if let Some(lark_cfg) = lark_cfg {
                let reply_target = target.clone();
                let reply_content = content.clone();
                let runtime = state.runtime.clone();
                let ss = state.secret_store.clone();
                runtime.spawn(async move {
                    let lark = crate::channel::lark::LarkChannel::new(lark_cfg, ss);
                    if let Err(e) = lark.send_message(&reply_target, &reply_content).await {
                        log::error!("Failed to send channel reply: {}", e);
                    }
                });
            }
            state.scheduler.mark_dirty();
        }

        AppEvent::ChannelStatusChanged { channel_id, status } => {
            log::info!(
                "Channel status changed: {} -> {:?}",
                channel_id,
                status.state
            );
            // Update the matching instance in the channels UI state.
            if let Some(inst) = state.channels_ui_state.instance_mut(&channel_id) {
                inst.connection_state = status.state;
                inst.last_error = status.last_error.clone();
                inst.messages_received = status.messages_received;
                inst.messages_sent = status.messages_sent;
            }
            // Also update legacy flat fields for backward compat.
            state.channels_ui_state.connection_state = status.state;
            state.channels_ui_state.last_error = status.last_error.clone();
            state.channels_ui_state.messages_received = status.messages_received;
            state.channels_ui_state.messages_sent = status.messages_sent;
            // Mirror connection state to titlebar UI state.
            state.ui_state.channel_connection_state = status.state;
            state.scheduler.mark_dirty();
        }
        AppEvent::ChannelTestResult { success, message } => {
            log::info!("Channel test result: success={}, msg={}", success, message);
            state.channels_ui_state.testing = false;
            let display = if success {
                format!("✓ {}", message)
            } else {
                format!("✗ {}", message)
            };
            state.channels_ui_state.test_status = Some((display, state.egui.ctx.input(|i| i.time)));
            state.scheduler.mark_dirty();
        }
        AppEvent::ChannelBotCheckResult { results } => {
            log::info!("Bot check completed: {} results", results.len());
            state.channels_ui_state.bot_check_results = Some(results);
            state.scheduler.mark_dirty();
        }
        AppEvent::ChannelTestResultById {
            instance_id,
            success,
            message,
        } => {
            log::info!(
                "Channel test result for '{}': success={}, msg={}",
                instance_id,
                success,
                message
            );
            if let Some(inst) = state.channels_ui_state.instance_mut(&instance_id) {
                inst.testing = false;
                let display = if success {
                    format!("✓ {}", message)
                } else {
                    format!("✗ {}", message)
                };
                inst.test_status = Some((display, state.egui.ctx.input(|i| i.time)));
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::ChannelBotCheckResultById {
            instance_id,
            results,
        } => {
            log::info!(
                "Bot check completed for '{}': {} results",
                instance_id,
                results.len()
            );
            if let Some(inst) = state.channels_ui_state.instance_mut(&instance_id) {
                inst.bot_check_results = Some(results);
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::ReloadSkills => {
            log::info!("Reloading external skills");
            if let Some(ref sm) = state.skills_manager {
                state.skill_registry.remove_by_category("external");
                let loader =
                    crate::ai::skills::loader::SkillLoader::new(sm.skills_dir().to_path_buf());
                match loader.load_all(&state.config) {
                    Ok(skills) => {
                        crate::ai::skills::loader::register_external_skills(
                            &mut state.skill_registry,
                            &skills,
                        );
                        state.loaded_external_skills = skills;
                    }
                    Err(e) => {
                        log::error!("Failed to reload skills: {}", e);
                    }
                }
                // Refresh the installed-list shown in the Skills panel.
                if let Ok(reg) = sm.load_registry() {
                    state.skills_panel_state.installed_list = reg.skills.into_iter().collect();
                }
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::SkillSearchResults(results) => {
            state.skills_panel_state.search_results = results;
            state.skills_panel_state.search_in_progress = false;
            state.scheduler.mark_dirty();
        }
        AppEvent::SkillSearchError(msg) => {
            state.skills_panel_state.search_in_progress = false;
            if !msg.is_empty() {
                state.skills_panel_state.last_error = Some(msg);
            }
            state.scheduler.mark_dirty();
        }

        // ── Extended skill events ────────────────────────────────────────
        AppEvent::FindFiles {
            query,
            cwd,
            max_results,
            request_id,
        } => {
            let mut files = Vec::new();
            let search_dir = std::path::Path::new(&cwd);
            if search_dir.is_dir() {
                find_files_recursive(search_dir, &query, max_results, &mut files);
            }
            let result = serde_json::json!({
                "files": files,
                "count": files.len(),
                "query": query,
                "cwd": cwd,
            });
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::StarChatBlock {
            session_id,
            message_id,
        } => {
            handle_ui_action(
                state,
                UiAction::ToggleStarred {
                    session_id,
                    message_id,
                },
                _event_loop,
            );
            state.scheduler.mark_dirty();
        }

        AppEvent::UnstarChatBlock {
            session_id,
            message_id,
        } => {
            handle_ui_action(
                state,
                UiAction::ToggleStarred {
                    session_id,
                    message_id,
                },
                _event_loop,
            );
            state.scheduler.mark_dirty();
        }

        AppEvent::ReferenceTerminalOutput {
            terminal_id,
            start_line,
            end_line,
            request_id,
        } => {
            let content = if let Some(session) = state.sessions.get(&terminal_id) {
                let history = session.state.history_size() as i32;
                let screen_lines = session.state.viewport_rows() as i32;
                let abs_start = start_line.unwrap_or(0);
                let abs_end = end_line.unwrap_or(history + screen_lines);
                let text = session.state.capture_text(abs_start, abs_end);
                let line_count = text.lines().count();
                serde_json::json!({
                    "content": text,
                    "lines": line_count,
                    "terminal_id": terminal_id,
                })
            } else {
                serde_json::json!({
                    "error": format!("Terminal {} not found", terminal_id),
                })
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(content);
            }
        }

        AppEvent::ReferenceBlockContent {
            session_id,
            message_id,
            request_id,
        } => {
            let result = if let Some(chat) = state.session_chats.get(&session_id) {
                if let Some(msg) = chat.history.get_by_id(message_id) {
                    serde_json::json!({
                        "content": msg.content,
                        "role": format!("{:?}", msg.role),
                        "starred": msg.importance == crate::ai::context::MessageImportance::Starred,
                        "token_count": msg.token_count,
                    })
                } else {
                    serde_json::json!({ "error": format!("Message {} not found", message_id) })
                }
            } else {
                serde_json::json!({ "error": format!("Session {} not found", session_id) })
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::AddContext {
            session_id,
            content,
            label,
        } => {
            if let Some(chat) = state.session_chats.get_mut(&session_id) {
                let token_count = state.token_counter.count(&content, "") as u32;
                let mut msg = crate::ai::context::ChatMessage::new(
                    crate::ai::context::MessageRole::System,
                    if let Some(lbl) = label {
                        format!("[Context: {}]\n{}", lbl, content)
                    } else {
                        content
                    },
                    crate::ai::context::MessageImportance::Ephemeral,
                    token_count,
                );
                msg.tool_call_id = None;
                chat.history.push(msg);
            }
        }

        AppEvent::RemoveContext {
            session_id,
            message_id,
        } => {
            if let Some(chat) = state.session_chats.get_mut(&session_id) {
                chat.history.remove_by_id(message_id);
            }
        }

        AppEvent::CompactContext {
            session_id,
            request_id,
        } => {
            // Simple compaction: prune to budget and report stats.
            let result = if let Some(chat) = state.session_chats.get_mut(&session_id) {
                let before = chat.history.len();
                let removed = chat.history.prune_to_budget(&state.token_counter, "");
                serde_json::json!({
                    "compacted_count": removed,
                    "freed_tokens": 0, // Approximate — prune_to_budget doesn't track exact freed count
                    "remaining_messages": before - removed,
                    "remaining_tokens": chat.history.total_tokens(),
                })
            } else {
                serde_json::json!({ "error": format!("Session {} not found", session_id) })
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::RenameTab { session_id, title } => {
            // Rename the session title.
            if let Some(session) = state.sessions.get_mut(&session_id) {
                session.title = title.clone();
            }
            // Also update the tab entry.
            for tab in state.ui_state.tabs.iter_mut() {
                match tab {
                    TabInfo::Chat {
                        session_id: sid,
                        title: t,
                    }
                    | TabInfo::Terminal {
                        session_id: sid,
                        title: t,
                    } if *sid == session_id => {
                        *t = title.clone();
                    }
                    _ => {}
                }
            }
            state.scheduler.mark_dirty();
        }

        _ => {}
    }
}
