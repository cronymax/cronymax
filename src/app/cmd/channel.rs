//! Channel/skills UI action handlers extracted from commands.rs

use crate::app::*;

pub(in crate::app) fn handle_ui_action_channel(
    state: &mut AppState,
    action: UiAction,
    #[allow(unused)] event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::ToggleLarkChannel(enabled) => {
            log::info!(
                "Lark channel toggled to {}",
                if enabled { "ON" } else { "OFF" }
            );
            if enabled {
                // Same as EnableClawMode: persist, then onboard or register.
                let claw_cfg = state.config.claw.get_or_insert_with(Default::default);
                claw_cfg.enabled = true;
                claw_cfg.migrate_legacy();
                if let Err(e) = crate::config::save_claw_enabled(true) {
                    log::error!("Failed to persist claw.enabled: {}", e);
                }
                // Sync the General settings toggle.
                state.general_ui_state.claw_mode_enabled = true;
                state.ui_state.claw_enabled = true;

                let has_channels = state
                    .config
                    .claw
                    .as_ref()
                    .map(|c| !c.channels.is_empty())
                    .unwrap_or(false);

                if !has_channels {
                    log::info!("No channels configured — launching onboarding wizard");

                    // Check if Lark credentials already exist via the credential system.
                    let has_existing_creds = crate::ai::skills::credentials::credential_resolve(
                        &state.secret_store,
                        "lark",
                        "app_secret",
                    )
                    .unwrap_or(false);

                    let mut wiz_state = crate::ui::settings::onboarding::OnboardingWizardState {
                        visible: true,
                        ..crate::ui::settings::onboarding::OnboardingWizardState::default()
                    };

                    if has_existing_creds {
                        wiz_state.status_message =
                            Some("Existing Lark credentials found — using stored values.".into());
                    }

                    if let Some(db) = &state.db_store {
                        if let Ok(Some(row)) = db.get_active_wizard(&state.secret_store) {
                            wiz_state =
                                crate::ui::settings::onboarding::OnboardingWizardState::from_db(
                                    &row,
                                );
                        } else if let Ok(id) = db.create_wizard(false) {
                            wiz_state.db_id = Some(id);
                        }
                    }
                    state.onboarding_wizard_state = Some(wiz_state);
                } else {
                    let claw_config = state.config.claw.clone().unwrap_or_default();
                    let proxy = state.proxy.clone();
                    let runtime = state.runtime.clone();
                    let ss = state.secret_store.clone();
                    runtime.spawn(async move {
                        let mut mgr = crate::channels::ChannelManager::new(proxy);
                        if let Err(e) =
                            crate::channels::register_channels(&mut mgr, &claw_config, ss).await
                        {
                            log::error!("Failed to register channels: {}", e);
                        }
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        }
                    });
                }
            } else {
                // Disable — same as DisableClawMode.
                if let Some(mut mgr) = state.channel_manager.take() {
                    let runtime = state.runtime.clone();
                    runtime.spawn(async move {
                        if let Err(e) = mgr.shutdown_all().await {
                            log::error!("Error shutting down channels: {}", e);
                        }
                    });
                }
                if let Some(ref mut claw_cfg) = state.config.claw {
                    claw_cfg.enabled = false;
                }
                if let Err(e) = crate::config::save_claw_enabled(false) {
                    log::error!("Failed to persist claw.enabled: {}", e);
                }
                // Sync the General settings toggle.
                state.general_ui_state.claw_mode_enabled = false;
                state.ui_state.claw_enabled = false;
                state.ui_state.channel_connection_state =
                    crate::channels::ConnectionState::Disconnected;
                // Dismiss any active wizard.
                state.onboarding_wizard_state = None;
            }
        }
        UiAction::StartOnboarding => {
            log::info!("Starting onboarding wizard (explicit)");
            let mut wiz_state = crate::ui::settings::onboarding::OnboardingWizardState {
                visible: true,
                ..crate::ui::settings::onboarding::OnboardingWizardState::default()
            };
            // Pre-fill from current config so re-run wizard has existing values.
            if let Some(claw) = &state.config.claw
                && let Some(lark_cfg) = claw
                    .channels
                    .iter()
                    .map(|c| match c {
                        crate::channels::config::ChannelConfig::Lark(cfg) => cfg,
                    })
                    .next()
            {
                wiz_state.app_id = lark_cfg.app_id.clone();
                wiz_state.app_secret_env = lark_cfg.app_secret_env.clone();
                wiz_state.api_base = lark_cfg.api_base.clone();
                wiz_state.allowed_users_text = lark_cfg.allowed_users.join(", ");
                wiz_state.profile_id = lark_cfg.profile_id.clone();
            }
            if let Some(db) = &state.db_store
                && let Ok(id) = db.create_wizard(false)
            {
                wiz_state.db_id = Some(id);
            }
            state.onboarding_wizard_state = Some(wiz_state);
        }
        UiAction::TestChannelConnection => {
            log::info!("Testing Lark channel connection (comprehensive diagnostic)");
            let lark_cfg_opt = state
                .config
                .claw
                .as_ref()
                .and_then(|c| {
                    c.channels
                        .iter()
                        .map(|ch| match ch {
                            crate::channels::config::ChannelConfig::Lark(cfg) => cfg.clone(),
                        })
                        .next()
                })
                .or_else(|| state.config.claw.as_ref().and_then(|c| c.lark.clone()));

            match lark_cfg_opt {
                Some(lark_cfg) => {
                    // Step 1: Validate config.
                    if let Err(e) = lark_cfg.validate() {
                        state.channels_ui_state.testing = false;
                        state.channels_ui_state.test_status = Some((
                            format!("✗ Config invalid: {}", e),
                            state.egui.ctx.input(|i| i.time),
                        ));
                    } else {
                        // Step 2: Check secret resolution (keychain + env var).
                        match lark_cfg.resolve_app_secret(&state.secret_store) {
                            Err(e) => {
                                state.channels_ui_state.testing = false;
                                state.channels_ui_state.test_status = Some((
                                    format!("✗ Secret: {}", e),
                                    state.egui.ctx.input(|i| i.time),
                                ));
                            }
                            Ok(_) => {
                                // Step 3: Run comprehensive async bot config check.
                                let proxy = state.proxy.clone();
                                let runtime = state.runtime.clone();
                                let ss = state.secret_store.clone();
                                runtime.spawn(async move {
                                    let lark =
                                        crate::channels::lark::LarkChannel::new(lark_cfg, ss);
                                    let results = lark.check_bot_config().await;

                                    // Build summary for the simple test_status field.
                                    let all_passed = results.iter().all(|r| r.passed);
                                    let failed_count = results.iter().filter(|r| !r.passed).count();
                                    let summary = if all_passed {
                                        "✓ All checks passed".to_string()
                                    } else {
                                        format!("⚠ {} issue(s) found", failed_count)
                                    };

                                    // Log all results.
                                    for r in &results {
                                        let icon = if r.passed { "✓" } else { "✗" };
                                        log::info!(
                                            "Bot check [{}] {}: {}",
                                            icon,
                                            r.label,
                                            r.detail
                                        );
                                    }

                                    let _ = proxy.send_event(AppEvent::ChannelTestResult {
                                        success: all_passed,
                                        message: summary,
                                    });
                                    let _ = proxy
                                        .send_event(AppEvent::ChannelBotCheckResult { results });
                                });
                            }
                        }
                    }
                }
                None => {
                    state.channels_ui_state.testing = false;
                    state.channels_ui_state.test_status = Some((
                        "✗ No Lark channel configured".into(),
                        state.egui.ctx.input(|i| i.time),
                    ));
                }
            }
        }
        UiAction::EnableClawMode => {
            log::info!("Enabling Claw mode");
            // Persist to config.
            let claw_cfg = state.config.claw.get_or_insert_with(Default::default);
            claw_cfg.enabled = true;
            // Migrate legacy [claw.lark] to [[claw.channels]] format.
            claw_cfg.migrate_legacy();

            if let Err(e) = crate::config::save_claw_enabled(true) {
                log::error!("Failed to persist claw.enabled: {}", e);
            }

            // Sync the Channels section toggle.
            state.channels_ui_state.lark_enabled = true;
            state.ui_state.claw_enabled = true;

            // If no channels configured, launch onboarding wizard.
            let has_channels = state
                .config
                .claw
                .as_ref()
                .map(|c| !c.channels.is_empty())
                .unwrap_or(false);

            if !has_channels {
                log::info!("No channels configured — launching onboarding wizard");
                let mut wiz_state = crate::ui::settings::onboarding::OnboardingWizardState {
                    visible: true,
                    ..crate::ui::settings::onboarding::OnboardingWizardState::default()
                };

                // Try to resume from DB.
                if let Some(db) = &state.db_store {
                    if let Ok(Some(row)) = db.get_active_wizard(&state.secret_store) {
                        wiz_state =
                            crate::ui::settings::onboarding::OnboardingWizardState::from_db(&row);
                    } else if let Ok(id) = db.create_wizard(false) {
                        wiz_state.db_id = Some(id);
                    }
                }
                state.onboarding_wizard_state = Some(wiz_state);
            } else {
                // Channels exist — register them.
                let claw_config = state.config.claw.clone().unwrap_or_default();
                let proxy = state.proxy.clone();
                let runtime = state.runtime.clone();
                let ss = state.secret_store.clone();
                runtime.spawn(async move {
                    let mut mgr = crate::channels::ChannelManager::new(proxy);
                    if let Err(e) =
                        crate::channels::register_channels(&mut mgr, &claw_config, ss).await
                    {
                        log::error!("Failed to register channels: {}", e);
                    }
                    // Keep the manager alive.
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                    }
                });
            }
        }
        UiAction::DisableClawMode => {
            log::info!("Disabling Claw mode");
            // Shutdown all channels.
            if let Some(mut mgr) = state.channel_manager.take() {
                let runtime = state.runtime.clone();
                runtime.spawn(async move {
                    if let Err(e) = mgr.shutdown_all().await {
                        log::error!("Error shutting down channels: {}", e);
                    }
                });
            }
            // Persist to config.
            if let Some(ref mut claw_cfg) = state.config.claw {
                claw_cfg.enabled = false;
            }
            if let Err(e) = crate::config::save_claw_enabled(false) {
                log::error!("Failed to persist claw.enabled: {}", e);
            }
            // Sync the Channels section toggle.
            state.channels_ui_state.lark_enabled = false;
            state.ui_state.claw_enabled = false;
            state.ui_state.channel_connection_state =
                crate::channels::ConnectionState::Disconnected;
        }
        UiAction::SaveChannelConfig => {
            // Save Lark channel config from UI state.
            let lark_config = state.channels_ui_state.to_lark_config();
            match crate::config::save_lark_config(&lark_config) {
                Ok(()) => {
                    log::info!("Saved Lark channel config to config.toml");
                    let claw_cfg = state.config.claw.get_or_insert_with(Default::default);
                    claw_cfg.lark = Some(lark_config);
                    state.channels_ui_state.save_status =
                        Some(("Saved!".into(), state.egui.ctx.input(|i| i.time)));
                }
                Err(e) => {
                    log::error!("Failed to save Lark config: {}", e);
                    state.channels_ui_state.save_status =
                        Some((format!("Error: {}", e), state.egui.ctx.input(|i| i.time)));
                }
            }
        }
        UiAction::SaveChannelConfigById { instance_id } => {
            log::info!("SaveChannelConfigById: {}", instance_id);
            // Serialize all instances and persist.
            let configs = state.channels_ui_state.to_channel_configs();
            let now = state.egui.ctx.input(|i| i.time);
            match crate::config::save_channel_configs(&configs) {
                Ok(()) => {
                    log::info!(
                        "Saved channel configs ({} instances) to config.toml",
                        configs.len()
                    );
                    if let Some(claw) = state.config.claw.as_mut() {
                        claw.channels = configs;
                    }
                    if let Some(inst) = state.channels_ui_state.instance_mut(&instance_id) {
                        inst.save_status = Some(("Saved!".into(), now));
                    }
                }
                Err(e) => {
                    log::error!("Failed to save channel configs: {}", e);
                    if let Some(inst) = state.channels_ui_state.instance_mut(&instance_id) {
                        inst.save_status = Some((format!("Error: {}", e), now));
                    }
                }
            }
        }
        UiAction::ToggleLarkChannelById {
            instance_id,
            enabled,
        } => {
            log::info!("ToggleLarkChannelById: {} -> {}", instance_id, enabled);
            if let Some(inst) = state.channels_ui_state.instance_mut(&instance_id) {
                inst.enabled = enabled;
            }
            // Persist the updated channel list.
            let configs = state.channels_ui_state.to_channel_configs();
            if let Some(claw) = state.config.claw.as_mut() {
                claw.channels = configs.clone();
            }
            if let Err(e) = crate::config::save_channel_configs(&configs) {
                log::error!("Failed to persist toggle for {}: {}", instance_id, e);
            }
        }
        UiAction::TestChannelConnectionById { instance_id } => {
            log::info!("TestChannelConnectionById: {}", instance_id);
            // Find the instance config from the in-memory config.
            let lark_cfg_opt = state.config.claw.as_ref().and_then(|c| {
                c.channels.iter().find_map(|ch| match ch {
                    crate::channels::config::ChannelConfig::Lark(cfg)
                        if cfg.instance_id == instance_id =>
                    {
                        Some(cfg.clone())
                    }
                    _ => None,
                })
            });

            match lark_cfg_opt {
                Some(lark_cfg) => {
                    if let Err(e) = lark_cfg.validate() {
                        if let Some(inst) = state.channels_ui_state.instance_mut(&instance_id) {
                            inst.testing = false;
                            inst.test_status = Some((
                                format!("✗ Config invalid: {}", e),
                                state.egui.ctx.input(|i| i.time),
                            ));
                        }
                    } else {
                        match lark_cfg.resolve_app_secret(&state.secret_store) {
                            Err(e) => {
                                if let Some(inst) =
                                    state.channels_ui_state.instance_mut(&instance_id)
                                {
                                    inst.testing = false;
                                    inst.test_status = Some((
                                        format!("✗ Secret: {}", e),
                                        state.egui.ctx.input(|i| i.time),
                                    ));
                                }
                            }
                            Ok(_) => {
                                let proxy = state.proxy.clone();
                                let runtime = state.runtime.clone();
                                let ss = state.secret_store.clone();
                                let iid = instance_id.clone();
                                runtime.spawn(async move {
                                    let lark =
                                        crate::channels::lark::LarkChannel::new(lark_cfg, ss);
                                    let results = lark.check_bot_config().await;
                                    let all_passed = results.iter().all(|r| r.passed);
                                    let failed_count = results.iter().filter(|r| !r.passed).count();
                                    let summary = if all_passed {
                                        "✓ All checks passed".to_string()
                                    } else {
                                        format!("⚠ {} issue(s) found", failed_count)
                                    };
                                    for r in &results {
                                        let icon = if r.passed { "✓" } else { "✗" };
                                        log::info!(
                                            "Bot check [{}] {} {}: {}",
                                            iid,
                                            icon,
                                            r.label,
                                            r.detail
                                        );
                                    }
                                    let _ = proxy.send_event(AppEvent::ChannelTestResultById {
                                        instance_id: iid.clone(),
                                        success: all_passed,
                                        message: summary,
                                    });
                                    let _ = proxy.send_event(AppEvent::ChannelBotCheckResultById {
                                        instance_id: iid,
                                        results,
                                    });
                                });
                            }
                        }
                    }
                }
                None => {
                    log::warn!(
                        "TestChannelConnectionById: no config found for '{}'",
                        instance_id
                    );
                    if let Some(inst) = state.channels_ui_state.instance_mut(&instance_id) {
                        inst.testing = false;
                        inst.test_status = Some((
                            "✗ No config found — save first".into(),
                            state.egui.ctx.input(|i| i.time),
                        ));
                    }
                }
            }
        }
        UiAction::AddLarkChannel => {
            let new_id = state.channels_ui_state.next_instance_id();
            log::info!("AddLarkChannel: creating instance '{}'", new_id);
            let inst = crate::ui::settings::channels::LarkInstanceState::new_empty(
                new_id,
                &state.channels_ui_state.secret_store,
            );
            state.channels_ui_state.instances.push(inst);
            // Persist the updated channel list.
            let configs = state.channels_ui_state.to_channel_configs();
            if let Some(claw) = state.config.claw.as_mut() {
                claw.channels = configs.clone();
            }
            if let Err(e) = crate::config::save_channel_configs(&configs) {
                log::error!("Failed to persist new channel instance: {}", e);
            }
        }
        UiAction::RemoveLarkChannel { instance_id } => {
            log::info!("RemoveLarkChannel: removing instance '{}'", instance_id);
            if state.channels_ui_state.remove_instance(&instance_id) {
                let configs = state.channels_ui_state.to_channel_configs();
                if let Some(claw) = state.config.claw.as_mut() {
                    claw.channels = configs.clone();
                }
                if let Err(e) = crate::config::save_channel_configs(&configs) {
                    log::error!("Failed to persist channel removal: {}", e);
                }
            } else {
                log::warn!("RemoveLarkChannel: instance '{}' not found", instance_id);
            }
        }
        _ => {}
    }
}
