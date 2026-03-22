//! Onboarding wizard and scheduler event handlers

use crate::app::*;

pub(in crate::app) fn handle_onboard_event(
    state: &mut AppState,
    event: AppEvent,
    _event_loop: &ActiveEventLoop,
) {
    match event {
        AppEvent::OnboardingStart {
            channel_type,
            instance_id,
            request_id,
        } => {
            let wizard = crate::ai::skills::onboarding::OnboardingWizardState {
                active: true,
                channel_type: channel_type.clone(),
                current_step: 1,
                total_steps: if channel_type == "lark" { 4 } else { 3 },
                completed_steps: Vec::new(),
                errors: Vec::new(),
                guided_mode: false,
                target_instance_id: instance_id.clone(),
            };
            if let Ok(mut shared) = state.shared_onboarding_state.lock() {
                *shared = Some(wizard.clone());
            }
            let result = serde_json::json!({
                "wizard_id": format!("{}_{}", channel_type, uuid::Uuid::new_v4()),
                "step": 1,
                "total_steps": wizard.total_steps,
                "description": "Open the Lark Developer Console and prepare a bot application",
                "instructions": "Open https://open.feishu.cn/app in the internal browser overlay, create or select your Custom App, then capture the App ID and App Secret. After that, enable the bot capability and import im:message, im:chat, and im:message.group_at_msg.",
                "recommended_skills": [
                    "onboard_lark_open_console",
                    "onboard_lark_store_credentials",
                    "onboard_lark_test_connection"
                ],
            });
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::OnboardingAdvanceStep { action, request_id } => {
            let result = if let Ok(mut shared) = state.shared_onboarding_state.lock() {
                if let Some(wizard) = shared.as_mut() {
                    match action.as_str() {
                        "next" => {
                            wizard
                                .completed_steps
                                .push(format!("step_{}", wizard.current_step));
                            wizard.current_step += 1;
                        }
                        "skip" => {
                            wizard.current_step += 1;
                        }
                        "retry" => {
                            wizard.errors.clear();
                        }
                        _ => {}
                    }
                    let complete = wizard.current_step > wizard.total_steps;
                    if complete {
                        wizard.active = false;
                    }
                    serde_json::json!({
                        "step": wizard.current_step,
                        "total_steps": wizard.total_steps,
                        "complete": complete,
                        "action": action,
                    })
                } else {
                    serde_json::json!({ "error": "No active onboarding wizard" })
                }
            } else {
                serde_json::json!({ "error": "Lock error" })
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::OnboardingStoreCredentials {
            app_id,
            app_secret,
            request_id,
        } => {
            let result = match crate::ai::skills::credentials::credential_store(
                &state.secret_store,
                "lark",
                "app_secret",
                &app_secret,
            ) {
                Ok(()) => {
                    // Also store app_id for credential_resolve() convenience.
                    let _ = crate::ai::skills::credentials::credential_store(
                        &state.secret_store,
                        "lark",
                        "app_id",
                        &app_id,
                    );
                    // Ensure the UI wizard state exists and has the app_id so that
                    // subsequent skill calls (e.g. test_connection) can locate the
                    // credentials.  This covers the skill-only flow where the user
                    // never opened the graphical wizard.
                    let wiz = state
                        .ui_state.onboarding_wizard_state
                        .get_or_insert_with(Default::default);
                    wiz.app_id = app_id.clone();
                    wiz.store_secret_in_keychain = true;
                    wiz.status_message = Some(format!(
                        "Stored app secret securely in keychain for {}.",
                        app_id
                    ));
                    wiz.error = None;
                    serde_json::json!({
                        "stored": true,
                        "service": "lark",
                        "message": "Credentials stored securely in keychain via credential system"
                    })
                }
                Err(e) => {
                    if let Some(wiz) = state.ui_state.onboarding_wizard_state.as_mut() {
                        wiz.error = Some(format!("Failed to store app secret: {}", e));
                    }
                    serde_json::json!({ "stored": false, "error": format!("Failed to store app_secret: {}", e) })
                }
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::OnboardingTestConnection { request_id } => {
            let pending = state.pending_results.clone();
            let secret_store = state.secret_store.clone();
            let rid = request_id.clone();
            let (app_id, app_secret_env, secret_storage) = state
                .ui_state.onboarding_wizard_state
                .as_ref()
                .map(|wiz| {
                    (
                        wiz.app_id.clone(),
                        wiz.app_secret_env.clone(),
                        wiz.selected_secret_storage(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        String::new(),
                        "LARK_APP_SECRET".to_string(),
                        crate::services::secret::SecretStorage::Auto,
                    )
                });
            state.runtime.spawn(async move {
                let result = if app_id.is_empty() {
                    serde_json::json!({
                        "connected": false,
                        "error": "No App ID set in the onboarding wizard"
                    })
                } else {
                    test_lark_connection(&secret_store, &app_id, &app_secret_env, &secret_storage)
                        .await
                };
                if let Ok(mut map) = pending.lock()
                    && let Some(tx) = map.remove(&rid)
                {
                    let _ = tx.send(result);
                }
            });
        }

        AppEvent::OnboardingFinalize { request_id } => {
            // Mirror the UI wizard's OnboardingWizardComplete logic:
            // Build LarkChannelConfig from the onboarding_wizard_state, save to
            // config.toml, start the channel WebSocket, and set claw_enabled.
            let wiz = match state.ui_state.onboarding_wizard_state.as_ref() {
                Some(w) if !w.app_id.is_empty() => w.clone(),
                _ => {
                    let result = serde_json::json!({
                        "finalized": false,
                        "error": "No onboarding wizard state or app_id is empty. Store credentials first."
                    });
                    if let Ok(mut map) = state.pending_results.lock()
                        && let Some(tx) = map.remove(&request_id)
                    {
                        let _ = tx.send(result);
                    }
                    return;
                }
            };

            let secret_storage = wiz.selected_secret_storage();
            let allowed_users: Vec<String> = wiz
                .allowed_users_text
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let lark_config = crate::channels::config::LarkChannelConfig {
                instance_id: "lark".into(),
                app_id: wiz.app_id.clone(),
                app_secret_env: if wiz.app_secret_env.is_empty() {
                    "LARK_APP_SECRET".into()
                } else {
                    wiz.app_secret_env.clone()
                },
                allowed_users,
                api_base: if wiz.api_base.is_empty() {
                    "https://open.feishu.cn".into()
                } else {
                    wiz.api_base.clone()
                },
                profile_id: if wiz.profile_id.is_empty() {
                    "default".into()
                } else {
                    wiz.profile_id.clone()
                },
                secret_storage,
            };
            let channel_cfg = crate::channels::config::ChannelConfig::Lark(lark_config.clone());
            let claw_cfg = state.config.claw.get_or_insert_with(Default::default);
            claw_cfg
                .channels
                .retain(|c| !matches!(c, crate::channels::config::ChannelConfig::Lark(_)));
            claw_cfg.channels.push(channel_cfg);
            claw_cfg.enabled = true;

            // Sync channels UI state.
            state.ui_state.channels_ui_state =
                crate::ui::settings::channels::ChannelsSettingsState::from_claw_config_with_store(
                    state.config.claw.as_ref(),
                    state.secret_store.clone(),
                );

            // Mark wizard complete in DB and clear UI state.
            if let Some(db) = &state.db_store
                && let Some(ref wiz_state) = state.ui_state.onboarding_wizard_state
                && let Some(db_id) = wiz_state.db_id
            {
                let _ = db.update_wizard_step(crate::ai::db::WizardStepUpdate {
                    id: db_id,
                    step: "completed",
                    lark_app_id: None,
                    oauth_token: None,
                    tenant_id: None,
                    secret_store: &state.secret_store,
                });
                let _ = db.clear_completed_wizards();
            }
            state.ui_state.onboarding_wizard_state = None;

            // Persist the Lark config to config.toml.
            if let Err(e) = crate::config::save_lark_config(&lark_config) {
                log::error!("Failed to save Lark config from skill onboarding: {}", e);
            }

            // Set claw_enabled in UI state.
            state.ui_state.claw_enabled = true;
            state.scheduler.mark_dirty();

            // Start the channel (connect WebSocket).
            let claw_config = state.config.claw.clone().unwrap_or_default();
            let proxy = state.proxy.clone();
            let runtime = state.runtime.clone();
            let ss = state.secret_store.clone();
            runtime.spawn(async move {
                let mut mgr = crate::channels::ChannelManager::new(proxy);
                if let Err(e) = crate::channels::register_channels(&mut mgr, &claw_config, ss).await
                {
                    log::error!("Failed to register channels after skill onboarding: {}", e);
                } else {
                    log::info!("Channel registered and WebSocket started after skill onboarding");
                }
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });

            log::info!("Onboarding finalized via skill — channel configured and started");

            let result = serde_json::json!({
                "finalized": true,
                "app_id": wiz.app_id,
                "message": "Channel configured and started. The Feishu icon should now appear in the titlebar."
            });
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::OnboardingBrowserAutomationFinished { success, message } => {
            if let Some(wiz) = state.ui_state.onboarding_wizard_state.as_mut() {
                wiz.loading = false;
                if success {
                    wiz.status_message = Some(message);
                    wiz.error = None;
                } else {
                    wiz.error = Some(message);
                }
            }
            state.scheduler.mark_dirty();
        }

        AppEvent::SchedulerCreate {
            name,
            cron,
            action_type,
            action_value,
            agent_name,
            enabled,
            run_once,
            request_id,
        } => {
            let result = {
                let task = crate::ai::scheduler::ScheduledTask {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.clone(),
                    cron: cron.clone(),
                    action_type,
                    action_value,
                    agent_name,
                    profile_id: "default".into(),
                    enabled,
                    run_once,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                let id = task.id.clone();
                match state.task_store.create(task) {
                    Ok(()) => serde_json::json!({
                        "id": id,
                        "name": name,
                        "cron": cron,
                        "cron_description": crate::ai::scheduler::cron_description(&cron),
                        "status": "created",
                    }),
                    Err(e) => serde_json::json!({ "error": format!("{}", e) }),
                }
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::SchedulerList { request_id } => {
            let tasks: Vec<serde_json::Value> = state
                .task_store
                .list()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "name": t.name,
                        "cron": t.cron,
                        "cron_description": crate::ai::scheduler::cron_description(&t.cron),
                        "action_type": t.action_type,
                        "enabled": t.enabled,
                    })
                })
                .collect();
            let count = tasks.len();
            let result = serde_json::json!({ "tasks": tasks, "count": count });
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::SchedulerGet {
            task_id,
            request_id,
        } => {
            let result = match state.task_store.get(&task_id) {
                Some(t) => serde_json::json!({
                    "id": t.id,
                    "name": t.name,
                    "cron": t.cron,
                    "cron_description": crate::ai::scheduler::cron_description(&t.cron),
                    "action_type": t.action_type,
                    "action_value": t.action_value,
                    "agent_name": t.agent_name,
                    "enabled": t.enabled,
                    "created_at": t.created_at,
                }),
                None => serde_json::json!({ "error": format!("Task '{}' not found", task_id) }),
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::SchedulerDelete {
            task_id,
            request_id,
        } => {
            let result = match state.task_store.delete(&task_id) {
                Ok(()) => serde_json::json!({ "status": "deleted", "deleted_id": task_id }),
                Err(e) => serde_json::json!({ "error": format!("{}", e) }),
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::SchedulerToggle {
            task_id,
            request_id,
        } => {
            let result = match state.task_store.toggle_enabled(&task_id) {
                Ok(enabled) => serde_json::json!({ "task_id": task_id, "enabled": enabled }),
                Err(e) => serde_json::json!({ "error": format!("{}", e) }),
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        AppEvent::SchedulerUpdate {
            task_id,
            name,
            cron,
            action_type,
            action_value,
            agent_name,
            enabled,
            request_id,
        } => {
            let result = if let Some(existing) = state.task_store.get(&task_id).cloned() {
                let updated = crate::ai::scheduler::ScheduledTask {
                    id: existing.id,
                    name: name.unwrap_or(existing.name),
                    cron: cron.unwrap_or(existing.cron),
                    action_type: action_type.unwrap_or(existing.action_type),
                    action_value: action_value.unwrap_or(existing.action_value),
                    agent_name: agent_name.unwrap_or(existing.agent_name),
                    profile_id: existing.profile_id,
                    enabled: enabled.unwrap_or(existing.enabled),
                    run_once: existing.run_once,
                    created_at: existing.created_at,
                };
                match state.task_store.update(&task_id, updated.clone()) {
                    Ok(()) => serde_json::json!({
                        "status": "updated",
                        "task": {
                            "id": updated.id,
                            "name": updated.name,
                            "cron": updated.cron,
                            "action_type": updated.action_type,
                            "enabled": updated.enabled,
                        },
                    }),
                    Err(e) => serde_json::json!({ "error": format!("{}", e) }),
                }
            } else {
                serde_json::json!({ "error": format!("Task '{}' not found", task_id) })
            };
            if let Ok(mut map) = state.pending_results.lock()
                && let Some(tx) = map.remove(&request_id)
            {
                let _ = tx.send(result);
            }
        }

        _ => {}
    }
}
