//! Onboarding wizard and skills UI action handlers

use crate::app::*;

const LARK_DEV_CONSOLE_URL: &str = "https://open.feishu.cn/app";

fn summarize_browser_automation(value: serde_json::Value) -> (bool, String) {
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return (false, format!("Lark console automation failed: {err}"));
    }
    if let Some(raw) = value.get("result").and_then(|v| v.as_str())
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw)
    {
        let log_lines = parsed
            .get("log")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .take(8)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_else(|| "Automation finished in the browser overlay".to_string());
        return (
            true,
            format!("Lark console automation finished: {log_lines}"),
        );
    }
    (
        true,
        "Lark console automation finished in the browser overlay".to_string(),
    )
}

fn build_lark_console_setup_script(request_id: &str, app_id: &str) -> String {
    let request_id_json = serde_json::to_string(request_id).unwrap_or_else(|_| "\"\"".into());
    let app_id_json = serde_json::to_string(app_id).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(async function() {{
  const requestId = {request_id_json};
  const appId = ({app_id_json} || '').toLowerCase();
  const recommended = 'im:message\nim:chat\nim:message.group_at_msg';
  const log = [];
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const visible = (el) => !!el && !!el.getBoundingClientRect && el.getBoundingClientRect().width > 0 && el.getBoundingClientRect().height > 0 && getComputedStyle(el).display !== 'none' && getComputedStyle(el).visibility !== 'hidden';
  const norm = (s) => (s || '').replace(/\s+/g, ' ').trim().toLowerCase();
  const nodes = () => Array.from(document.querySelectorAll('button,a,[role="button"],label,span,div'));
  const findByText = (patterns) => nodes().find((el) => visible(el) && patterns.some((p) => norm(el.innerText).includes(p)));
  const clickByText = (patterns, label) => {{
    const el = findByText(patterns);
    if (!el) {{
      log.push(`missing ${{label}}`);
      return false;
    }}
    el.scrollIntoView({{ block: 'center', inline: 'center' }});
    el.click();
    log.push(`clicked ${{label}}`);
    return true;
  }};
  const fillImportField = (value) => {{
    const field = Array.from(document.querySelectorAll('textarea,input[type="text"],input:not([type])')).find((el) => visible(el));
    if (!field) {{
      log.push('missing permissions import field');
      return false;
    }}
    field.focus();
    field.value = value;
    field.dispatchEvent(new Event('input', {{ bubbles: true }}));
    field.dispatchEvent(new Event('change', {{ bubbles: true }}));
    log.push('filled batch import permissions');
    return true;
  }};
  for (let i = 0; i < 20 && (!document.body || !document.body.innerText); i++) {{
    await sleep(250);
  }}
  if (appId) {{
    const appNode = findByText([appId]);
    if (appNode) {{
      appNode.click();
      log.push(`selected app ${{appId}}`);
      await sleep(900);
    }} else {{
      log.push(`app ${{appId}} not found on current page; continuing in current console view`);
    }}
  }}
  clickByText(['bot', '机器人'], 'bot section');
  await sleep(700);
  clickByText(['enable bot', 'turn on bot', '开启机器人', '启用机器人'], 'enable bot');
  await sleep(700);
  clickByText(['permissions & scopes', 'permissions', 'scope', '权限', '权限与范围'], 'permissions section');
  await sleep(700);
  if (clickByText(['batch import', 'import permissions', '批量导入', '导入权限'], 'batch import')) {{
    await sleep(700);
  }}
  fillImportField(recommended);
  await sleep(300);
  clickByText(['confirm', 'import', 'save', 'apply', '确定', '导入', '保存'], 'confirm import');
  await sleep(500);
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{
      request_id: requestId,
      result: JSON.stringify({{ ok: true, log: log, current_url: window.location.href || '' }}),
      error: null
    }}
  }});
}})().catch((e) => {{
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: {request_id_json}, result: null, error: e.message }}
  }});
}});"#,
        request_id_json = request_id_json,
        app_id_json = app_id_json,
    )
}

fn start_lark_console_automation(state: &mut AppState, event_loop: &ActiveEventLoop, app_id: &str) {
    open_webview(state, LARK_DEV_CONSOLE_URL, event_loop);

    let request_id = uuid::Uuid::new_v4().to_string();
    let script = build_lark_console_setup_script(&request_id, app_id);
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(mut map) = state.pending_results.lock() {
        map.insert(request_id.clone(), tx);
    }

    let proxy = state.proxy.clone();
    state.runtime.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let _ = proxy.send_event(AppEvent::InjectScript {
            webview_id: 0,
            script,
            request_id: request_id.clone(),
        });
        let (success, message) =
            match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
                Ok(Ok(value)) => summarize_browser_automation(value),
                Ok(Err(_)) => (
                    false,
                    "Lark console automation channel closed unexpectedly".into(),
                ),
                Err(_) => (false, "Lark console automation timed out after 15s".into()),
            };
        let _ =
            proxy.send_event(AppEvent::OnboardingBrowserAutomationFinished { success, message });
    });
}

pub(in crate::app) fn handle_ui_action_onboard(
    state: &mut AppState,
    action: UiAction,
    #[allow(unused)] event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::OnboardingWizardStepChanged { step } => {
            // Persist wizard step to SQLite for resumability.
            if let Some(db) = &state.db_store
                && let Some(ref mut wiz) = state.onboarding_wizard_state
                && let Some(db_id) = wiz.db_id
                && let Err(e) = db.update_wizard_step(crate::ai::db::WizardStepUpdate {
                    id: db_id,
                    step: &step,
                    lark_app_id: None,
                    oauth_token: None,
                    tenant_id: None,
                    secret_store: &state.secret_store,
                })
            {
                log::error!("Failed to persist wizard step: {}", e);
            }
        }
        UiAction::OnboardingAutomateLarkSetup { app_id } => {
            if app_id.trim().is_empty() {
                if let Some(wiz) = state.onboarding_wizard_state.as_mut() {
                    wiz.loading = false;
                    wiz.error = Some(
                        "Enter the App ID first so the overlay can target the right Lark app."
                            .into(),
                    );
                }
            } else {
                if let Some(wiz) = state.onboarding_wizard_state.as_mut() {
                    wiz.loading = true;
                    wiz.error = None;
                    wiz.status_message = Some(
                        "Opening the Lark Developer Console in the browser overlay and applying bot/permission setup…".into(),
                    );
                }
                start_lark_console_automation(state, event_loop, &app_id);
            }
        }
        UiAction::OnboardingWizardComplete {
            app_id,
            app_secret,
            app_secret_env,
            api_base,
            allowed_users,
            profile_id,
            secret_storage,
        } => {
            if secret_storage == crate::services::secret::SecretStorage::Keychain
                && app_secret.as_deref().is_none_or(|s| s.trim().is_empty())
            {
                if let Some(wiz) = state.onboarding_wizard_state.as_mut() {
                    wiz.loading = false;
                    wiz.error = Some(
                        "Paste the App Secret before finishing so it can be stored in keychain."
                            .into(),
                    );
                }
                return;
            }
            if secret_storage != crate::services::secret::SecretStorage::Keychain
                && app_secret_env.trim().is_empty()
            {
                if let Some(wiz) = state.onboarding_wizard_state.as_mut() {
                    wiz.loading = false;
                    wiz.error = Some(
                        "Provide an App Secret env var name, or switch to keychain storage.".into(),
                    );
                }
                return;
            }
            if let Some(secret) = app_secret.as_deref().filter(|s| !s.trim().is_empty())
                && secret_storage == crate::services::secret::SecretStorage::Keychain
            {
                if let Err(e) = crate::ai::skills::credentials::credential_store(
                    &state.secret_store,
                    "lark",
                    "app_secret",
                    secret,
                ) {
                    if let Some(wiz) = state.onboarding_wizard_state.as_mut() {
                        wiz.loading = false;
                        wiz.error = Some(format!("Failed to store app secret in keychain: {}", e));
                    }
                    return;
                }
                // Also store the app_id for credential_resolve() convenience.
                let _ = crate::ai::skills::credentials::credential_store(
                    &state.secret_store,
                    "lark",
                    "app_id",
                    &app_id,
                );
            }

            // Create channel config from wizard results and save.
            let lark_config = crate::channels::config::LarkChannelConfig {
                instance_id: "lark".into(),
                app_id: app_id.clone(),
                app_secret_env: app_secret_env.clone(),
                allowed_users: allowed_users.clone(),
                api_base: api_base.clone(),
                profile_id: profile_id.clone(),
                secret_storage: secret_storage.clone(),
            };
            let channel_cfg = crate::channels::config::ChannelConfig::Lark(lark_config.clone());
            let claw_cfg = state.config.claw.get_or_insert_with(Default::default);
            // Replace existing Lark channel(s) to avoid duplicates on re-run.
            claw_cfg
                .channels
                .retain(|c| !matches!(c, crate::channels::config::ChannelConfig::Lark(_)));
            claw_cfg.channels.push(channel_cfg);
            claw_cfg.enabled = true;

            // Sync channels UI state with the new config.
            state.channels_ui_state =
                crate::ui::settings::channels::ChannelsSettingsState::from_claw_config_with_store(
                    state.config.claw.as_ref(),
                    state.secret_store.clone(),
                );

            // Mark wizard complete in DB and clear UI state.
            if let Some(db) = &state.db_store
                && let Some(ref wiz) = state.onboarding_wizard_state
                && let Some(db_id) = wiz.db_id
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
            state.onboarding_wizard_state = None;

            // Persist the Lark config to config.toml.
            if let Err(e) = crate::config::save_lark_config(&lark_config) {
                log::error!("Failed to save Lark config from wizard: {}", e);
            }

            // ── Actually start the channel (connect WebSocket) ───────────
            let claw_config = state.config.claw.clone().unwrap_or_default();
            let proxy = state.proxy.clone();
            let runtime = state.runtime.clone();
            let ss = state.secret_store.clone();
            runtime.spawn(async move {
                let mut mgr = crate::channels::ChannelManager::new(proxy);
                if let Err(e) = crate::channels::register_channels(&mut mgr, &claw_config, ss).await
                {
                    log::error!("Failed to register channels after wizard: {}", e);
                } else {
                    log::info!("Channel registered and WebSocket started after wizard completion");
                }
                // Keep the manager alive so the WS loop keeps running.
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });

            log::info!("Onboarding wizard completed — channel configured and started");
        }
        UiAction::ToggleStarred {
            session_id,
            message_id,
        } => {
            // Toggle starred state in DB.
            if let Some(db) = &state.db_store {
                // Check current state and toggle.
                let starred_ids = db.get_starred_blocks(session_id).unwrap_or_default();
                let is_starred = starred_ids.contains(&message_id);
                if let Err(e) = db.set_starred_block(session_id, message_id, !is_starred) {
                    log::error!("Failed to toggle starred block: {}", e);
                }
            }
        }
        UiAction::CloseChannel(channel_id) => {
            tiles::remove_channel_pane(&mut state.tile_tree, &channel_id);
            log::info!("Closed channel tab: {channel_id}");
        }
        UiAction::OpenChannelTab {
            channel_id,
            channel_name,
        } => {
            tiles::add_channel_tab(&mut state.tile_tree, &channel_id, &channel_name);
            log::info!("Opened channel tab: {channel_name} ({channel_id})");
        }
        UiAction::InstallSkill(slug) => {
            log::info!("Install skill requested: {slug}");
            if let Some(ref sm) = state.skills_manager {
                let sm_clone_skills_dir = sm.skills_dir().to_path_buf();
                let sm_api_base = sm.api_base().to_string();
                let proxy = state.proxy.clone();
                let _config = state.config.clone();
                state.runtime.spawn(async move {
                    let sm = crate::ai::skills::manager::SkillsManager::new(
                        sm_clone_skills_dir,
                        sm_api_base,
                    );
                    match sm.install(&slug).await {
                        Ok(skill) => {
                            log::info!("Installed skill '{}' successfully", skill.frontmatter.name);
                            // Trigger reload via event.
                            let _ = proxy.send_event(AppEvent::ReloadSkills);
                        }
                        Err(e) => {
                            log::error!("Failed to install skill '{}': {}", slug, e);
                        }
                    }
                });
            }
        }
        UiAction::UninstallSkill(name) => {
            log::info!("Uninstall skill requested: {name}");
            if let Some(ref sm) = state.skills_manager {
                if let Err(e) = sm.uninstall(&name) {
                    log::error!("Failed to uninstall skill '{}': {}", name, e);
                } else {
                    // Remove from loaded skills and skill registry.
                    state
                        .loaded_external_skills
                        .retain(|s| s.frontmatter.name != name);
                    state.skill_registry.remove_by_category("external");
                    crate::ai::skills::loader::register_external_skills(
                        &mut state.skill_registry,
                        &state.loaded_external_skills,
                    );
                }
            }
        }
        UiAction::ToggleSkill { name, enabled } => {
            log::info!("Toggle skill '{name}' → enabled={enabled}");
            if let Some(ref sm) = state.skills_manager
                && let Err(e) = sm.set_enabled(&name, enabled)
            {
                log::error!("Failed to toggle skill '{}': {}", name, e);
            }
        }
        UiAction::SearchSkills(query) => {
            log::info!("Search ClawHub: {query}");
            if let Some(ref sm) = state.skills_manager {
                let sm_skills_dir = sm.skills_dir().to_path_buf();
                let sm_api_base = sm.api_base().to_string();
                let proxy = state.proxy.clone();
                state.runtime.spawn(async move {
                    let sm =
                        crate::ai::skills::manager::SkillsManager::new(sm_skills_dir, sm_api_base);
                    match sm.search(&query).await {
                        Ok(results) => {
                            let _ = proxy.send_event(AppEvent::SkillSearchResults(results));
                        }
                        Err(e) => {
                            log::warn!("ClawHub search failed: {e}");
                            let _ = proxy.send_event(AppEvent::SkillSearchError(format!(
                                "Search failed: {e}"
                            )));
                        }
                    }
                });
            }
        }
        UiAction::ReloadSkills => {
            log::info!("Reload skills from filesystem");
            if let Some(ref sm) = state.skills_manager {
                state.skill_registry.remove_by_category("external");
                match sm.load_and_register(&mut state.skill_registry, &state.config) {
                    Ok(count) => {
                        log::info!("Reloaded {} external skills", count);
                    }
                    Err(e) => {
                        log::error!("Failed to reload skills: {}", e);
                    }
                }
            }
        }
        UiAction::ToggleSkillsPanel => {
            state.skills_panel_state.open = !state.skills_panel_state.open;
        }
        _ => {}
    }
}
