//! Window/profile/settings UI action handlers extracted from commands.rs

use crate::renderer::scheduler::RenderSchedule;

use crate::ui::{BrowserViewMode, Ui, UiAction, model::AppCtx, tiles};

impl Ui {
    pub(crate) fn handle_ui_action_settings(
        &mut self,
        ctx: &mut AppCtx<'_>,
        action: UiAction,
        #[allow(unused)] event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        match action {
            UiAction::PinTab(sid) if !ctx.ui_state.pinned_tabs.contains(&sid) => {
                ctx.ui_state.pinned_tabs.push(sid);
                // Re-activate the pinned pane so it stays focused instead
                // of having egui_tiles switch to the next visible tab.
                if self.browser_tabs.iter().any(|wt| wt.browser.id == sid) {
                    tiles::activate_browser_view_tab(&mut self.tile_tree, sid);
                } else {
                    tiles::activate_terminal_tab(&mut self.tile_tree, sid);
                }
                log::info!("Pinned tab session {} (hidden from tab bar)", sid);
                ctx.scheduler.mark_dirty();
            }
            UiAction::UnpinTab(sid) => {
                ctx.ui_state.pinned_tabs.retain(|&s| s != sid);
                log::info!("Unpinned tab session {} (shown in tab bar)", sid);
                ctx.scheduler.mark_dirty();
            }
            UiAction::StartWindowDrag => {
                // Delegate to the OS for smooth, flicker-free window movement.
                let _ = self.frame.window.drag_window();
            }
            UiAction::CloseWindow => {
                // Request exit — the event loop will handle cleanup.
                log::info!("CloseWindow action: requesting exit");
                // We store a flag; the event_loop.exit() call happens in the event handler.
                // For now, just close all sessions and drop.
                std::process::exit(0);
            }
            UiAction::Minimize => {
                self.frame.window.set_minimized(true);
            }
            UiAction::ToggleMaximize => {
                let is_max = self.frame.window.is_maximized();
                self.frame.window.set_maximized(!is_max);
            }
            UiAction::PopOutOverlay => {
                // Promote the active webview to a floating overlay.
                //
                // If already an Overlay → promote to Independent (own egui browser).
                // If Docked → promote to Overlay:
                //   1. Create Modal overlay (child window + wgpu surface + egui)
                //   2. Reparent wry WebView to the overlay panel
                //   3. Remove from tile tree, release tile space
                //   4. Update mode + z-order manager
                if let Some(idx) = ctx.ui_state.active_browser
                    && let Some(tab) = self.browser_tabs.get_mut(idx)
                {
                    let wid = tab.browser.id;
                    let url = tab.browser.url.clone();

                    match tab.mode {
                        BrowserViewMode::Overlay => {
                            // Already overlay → promote to independent.
                            self.browser_manager
                                .promote_to_independent_with_url(wid, &url);
                            log::info!("Promoted overlay webview {} to independent", wid);
                        }
                        BrowserViewMode::Docked => {
                            // Docked → Overlay: create modal overlay and reparent.
                            let win_size = self.frame.window.inner_size();
                            let scale = self.frame.window.scale_factor() as f32;
                            let address_bar_h = self.styles.address_bar_height();

                            // Default overlay position: centered, 70% of window size.
                            let panel_lw = (win_size.width as f32 / scale) * 0.7;
                            let panel_lh = (win_size.height as f32 / scale) * 0.7;
                            let panel_lx = ((win_size.width as f32 / scale) - panel_lw) / 2.0;
                            let panel_ly = ((win_size.height as f32 / scale) - panel_lh) / 2.0;

                            let rect = crate::renderer::panel::LogicalRect {
                                x: panel_lx,
                                y: panel_ly,
                                w: panel_lw,
                                h: panel_lh,
                                scale,
                            };

                            match crate::ui::overlay::Modal::new(
                                &self.frame.window,
                                Some(event_loop),
                                &self.frame.gpu,
                                rect,
                            ) {
                                Ok(modal) => {
                                    let wv_bounds = crate::app::browser::webview_bounds_below_bar(
                                        rect,
                                        address_bar_h,
                                    );
                                    tab.browser.view.reparent_to_window(&modal.panel);
                                    tab.browser.view.set_viewport(wv_bounds);
                                    tab.browser.view.set_visible(true);

                                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                                    {
                                        tab.overlay = Some(modal);
                                    }
                                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                                    {
                                        let _ = modal;
                                    }
                                    tab.browser.address_bar.url = url.clone();
                                    tab.mode = BrowserViewMode::Overlay;
                                    tab.overlay_origin = (panel_lx, panel_ly);

                                    // Remove from tile tree.
                                    tiles::remove_browser_view_pane(&mut self.tile_tree, wid);

                                    // Update z-order manager.
                                    self.browser_manager.promote_to_overlay(wid);

                                    log::info!("Promoted docked webview {} to overlay", wid);
                                }
                                Err(e) => {
                                    log::warn!("Failed to create overlay panel for pop-out: {e}");
                                }
                            }

                            ctx.scheduler.mark_dirty();
                        }
                    }
                }
            }
            UiAction::BringOverlayToFront(wid) => {
                self.browser_manager.bring_to_front(wid);
                // Also set it as the active webview.
                if let Some(idx) = self.browser_tabs.iter().position(|wt| wt.browser.id == wid) {
                    self.active_browser = idx;
                    ctx.ui_state.active_browser = Some(idx);
                }
                // Dismiss any stale tooltip from the previously-frontmost overlay.
                self.float_panel_state.clear();
                log::info!("Brought overlay webview {} to front", wid);
            }
            UiAction::OpenSettings => {
                ctx.ui_state.settings_state.open = true;
                ctx.scheduler.mark_dirty();
                log::info!("OpenSettings action received");
                // Refresh installed skills list for the panel.
                if let Some(sm) = ctx.skills_manager
                    && let Ok(reg) = sm.load_registry()
                {
                    ctx.ui_state.skills_panel_state.installed_list =
                        reg.skills.into_iter().collect();
                }
            }
            UiAction::CloseSettings => {
                ctx.ui_state.settings_state.open = false;
                ctx.scheduler.mark_dirty();
                log::info!("CloseSettings action received");
            }
            UiAction::OpenOverlay => {
                let url = ctx
                    .config
                    .webview
                    .default_url
                    .clone()
                    .unwrap_or_else(|| "https://google.com".to_string());
                log::info!("OpenOverlay action: opening {}", url);
                crate::app::open_browser(self, ctx, &url, event_loop);
            }
            UiAction::RelaunchApp => {
                log::info!("Relaunching application for sandbox policy update…");
                match std::env::current_exe() {
                    Ok(exe) => {
                        let _ = std::process::Command::new(&exe)
                            .args(std::env::args().skip(1))
                            .spawn();
                        std::process::exit(0);
                    }
                    Err(e) => {
                        log::error!("Failed to determine current executable for relaunch: {}", e);
                    }
                }
            }
            UiAction::NewWindowWithProfile(pid) => {
                log::info!("Opening new window with profile '{}'", pid);
                match std::env::current_exe() {
                    Ok(exe) => {
                        let mut cmd = std::process::Command::new(&exe);
                        // Forward existing args, but strip any prior --profile to avoid duplicates.
                        let args: Vec<String> = std::env::args().skip(1).collect();
                        let mut skip_next = false;
                        for arg in &args {
                            if skip_next {
                                skip_next = false;
                                continue;
                            }
                            if arg == "--profile" {
                                skip_next = true; // skip the value that follows
                                continue;
                            }
                            if arg.starts_with("--profile=") {
                                continue;
                            }
                            cmd.arg(arg);
                        }
                        cmd.arg("--profile").arg(&pid);
                        if let Err(e) = cmd.spawn() {
                            log::error!("Failed to spawn new window with profile '{}': {}", pid, e);
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to determine current executable for new window: {}",
                            e
                        );
                    }
                }
            }
            UiAction::SwitchModel {
                session_id,
                provider,
                model,
                display_label,
            } => {
                log::info!(
                    "SwitchModel for session {}: {} / {}",
                    session_id,
                    provider,
                    model
                );
                // Update the per-session model override.
                if let Some(chat) = ctx.session_chats.get_mut(&session_id) {
                    let llm_provider = match provider.as_str() {
                        "OpenAI" => crate::ai::client::LlmProvider::OpenAI,
                        "Anthropic" => crate::ai::client::LlmProvider::Anthropic,
                        "GitHub Copilot" => crate::ai::client::LlmProvider::Copilot,
                        "Ollama (local)" => crate::ai::client::LlmProvider::Ollama,
                        _ => crate::ai::client::LlmProvider::Custom,
                    };
                    chat.model_override = Some(crate::ai::client::ModelSelection {
                        provider: llm_provider,
                        model,
                        display_label,
                    });
                }
            }
            UiAction::OpenWebviewTab(url) => {
                log::info!("OpenWebviewTab (from skill): {}", url);
                crate::app::open_browser(self, ctx, &url, event_loop);
            }
            UiAction::CloseWebviewTab(wid) => {
                // Close the webview tab by ID.
                self.handle_ui_action(ctx, UiAction::CloseWebview(wid), event_loop);
            }
            UiAction::InstallAgent => {
                // Open native file dialog to pick agent directory.
                let dialog = rfd::FileDialog::new().set_title("Select Agent Directory");
                if let Some(path) = dialog.pick_folder() {
                    match ctx.agent_registry.install(&path) {
                        Ok(name) => {
                            log::info!("Agent '{}' installed from {:?}", name, path);
                        }
                        Err(e) => {
                            log::error!("Failed to install agent: {}", e);
                        }
                    }
                }
            }
            UiAction::UninstallAgent(name) => match ctx.agent_registry.uninstall(&name) {
                Ok(()) => {
                    log::info!("Agent '{}' uninstalled", name);
                }
                Err(e) => {
                    log::error!("Failed to uninstall agent '{}': {}", name, e);
                }
            },
            UiAction::ToggleAgent(name) => {
                let entry = ctx.agent_registry.installed.iter().find(|a| a.name == name);
                let currently_enabled = entry.map(|e| e.enabled).unwrap_or(false);
                let result = if currently_enabled {
                    ctx.agent_registry.disable(&name)
                } else {
                    ctx.agent_registry.enable(&name)
                };
                if let Err(e) = result {
                    log::error!("Failed to toggle agent '{}': {}", name, e);
                }
            }
            UiAction::CreateProfile => {
                let name = ctx.ui_state.profiles_ui_state.edit_name.clone();
                let mut mgr = ctx.profile_manager.lock().unwrap();
                match mgr.create_profile(&name) {
                    Ok(p) => {
                        let new_id = p.id.clone();
                        log::info!("Created profile '{}'", new_id);
                        ctx.ui_state.profiles_ui_state.selected_profile_id = Some(new_id);
                        ctx.ui_state.profiles_ui_state.creating_new = false;
                    }
                    Err(e) => log::error!("Failed to create profile: {}", e),
                }
            }
            UiAction::SaveProfile(pid) => {
                let name = ctx.ui_state.profiles_ui_state.edit_name.clone();
                let sandbox = ctx.ui_state.profiles_ui_state.sandbox_policy_from_state();
                let mut mgr = ctx.profile_manager.lock().unwrap();
                let mut ok = true;
                if let Err(e) = mgr.edit_profile(&pid, Some(&name), Some(sandbox.clone())) {
                    log::error!("Failed to save profile '{}': {}", pid, e);
                    ok = false;
                } else {
                    log::info!("Saved profile '{}' to profile.toml (incl. sandbox)", pid);
                }
                // Also save sandbox to policy.toml for backward compatibility.
                let policy_path = mgr.profile_dir(&pid).join("policy.toml");
                if let Err(e) = sandbox.save_to_file(&policy_path) {
                    log::warn!("Failed to save policy.toml for '{}': {}", pid, e);
                }
                // Show feedback in the UI.
                let msg = if ok {
                    "Saved!"
                } else {
                    "Save failed — check logs"
                };
                ctx.ui_state.profiles_ui_state.save_status =
                    Some((msg.into(), std::time::Instant::now()));

                // Detect if sandbox rules changed from what was applied at PTY spawn.
                // If they differ, show a relaunch dialog so the OS-level sandbox is re-applied.
                if ok && sandbox != *ctx.applied_sandbox_policy {
                    ctx.ui_state.profiles_ui_state.show_relaunch_dialog = true;
                    log::info!(
                        "Sandbox policy changed — relaunch required for OS-level enforcement"
                    );
                }
            }
            UiAction::DuplicateProfile(pid) => {
                let new_name = format!("{} (copy)", ctx.ui_state.profiles_ui_state.edit_name);
                let mut mgr = ctx.profile_manager.lock().unwrap();
                match mgr.duplicate_profile(&pid, &new_name) {
                    Ok(new_id) => {
                        log::info!("Duplicated profile '{}' → '{}'", pid, new_id);
                        ctx.ui_state.profiles_ui_state.selected_profile_id = Some(new_id);
                        ctx.ui_state.profiles_ui_state.fields_loaded_for = None;
                    }
                    Err(e) => log::error!("Failed to duplicate profile '{}': {}", pid, e),
                }
            }
            UiAction::DeleteProfile(pid) => {
                let mut mgr = ctx.profile_manager.lock().unwrap();
                // If deleting the active profile, switch to another profile first.
                if mgr.active_profile_id.as_deref() == Some(&pid) {
                    let other_id = mgr.profiles.keys().find(|k| k.as_str() != pid).cloned();
                    if let Some(other) = other_id {
                        if let Err(e) = mgr.set_active(&other) {
                            log::error!("Failed to switch active profile before delete: {}", e);
                        } else {
                            log::info!(
                                "Auto-switched active profile to '{}' before deleting '{}'",
                                other,
                                pid
                            );
                        }
                    }
                }
                if let Err(e) = mgr.delete_profile(&pid) {
                    log::error!("Failed to delete profile '{}': {}", pid, e);
                    ctx.ui_state.profiles_ui_state.save_status =
                        Some((format!("Delete failed: {}", e), std::time::Instant::now()));
                }
            }
            UiAction::SetActiveProfile(pid) => {
                let mut mgr = ctx.profile_manager.lock().unwrap();
                // Load the new profile's sandbox policy *before* switching.
                let new_sandbox = mgr.sandbox_policy(&pid).unwrap_or_else(|_| {
                    crate::profile::sandbox::policy::SandboxPolicy::from_default()
                });
                match mgr.set_active(&pid) {
                    Ok(()) => {
                        log::info!("Switched active profile to '{}'", pid);
                        drop(mgr); // release lock before mutating ui state
                        // If the sandbox policy differs from the one applied at process
                        // launch, prompt the user to relaunch.
                        if new_sandbox != *ctx.applied_sandbox_policy {
                            ctx.ui_state.show_profile_relaunch_dialog = true;
                            log::info!(
                                "Sandbox policy changed (profile '{}') — relaunch dialog shown",
                                pid
                            );
                        }
                    }
                    Err(e) => log::error!("Failed to switch profile: {}", e),
                }
            }
            UiAction::SaveProviders => {
                let providers = ctx.ui_state.providers_ui_state.to_provider_configs();
                match crate::config::save_providers_to_config(&providers) {
                    Ok(()) => {
                        log::info!("Saved {} LLM providers to config.toml", providers.len());
                        // Update the in-memory config too.
                        if ctx.config.ai.is_none() {
                            ctx.config.ai = Some(crate::config::AiConfig::default());
                        }
                        if let Some(ai) = ctx.config.ai.as_mut() {
                            ai.providers = Some(providers.clone());
                        }
                        // Feed updated providers to the LLM client.
                        if let Some(client) = ctx.llm_client.as_mut() {
                            client.set_configured_providers(providers);
                            // Re-fetch available models with the new provider config.
                            client.fetch_available_models(ctx.proxy.clone(), ctx.runtime);
                        }
                        ctx.ui_state.providers_ui_state.save_status =
                            Some(("Saved!".into(), std::time::Instant::now()));
                    }
                    Err(e) => {
                        log::error!("Failed to save providers: {}", e);
                        ctx.ui_state.providers_ui_state.save_status =
                            Some((format!("Error: {}", e), std::time::Instant::now()));
                    }
                }
            }
            UiAction::CreateScheduledTask
            | UiAction::SaveScheduledTask(_)
            | UiAction::DeleteScheduledTask(_)
            | UiAction::ToggleScheduledTask(_) => {
                self.handle_scheduled_task_action(ctx, action);
            }
            _ => {}
        }
    }

    /// Scheduled-task handler — thin adapter that reads from `Ui` + `AppCtx`.
    fn handle_scheduled_task_action(&mut self, ctx: &mut AppCtx<'_>, action: UiAction) {
        match action {
            UiAction::CreateScheduledTask => {
                let ui_st = &ctx.ui_state.scheduler_ui_state;
                let task = crate::ai::scheduler::ScheduledTask {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: ui_st.edit_name.clone(),
                    cron: ui_st.edit_cron.clone(),
                    action_type: ui_st.edit_action_type.clone(),
                    action_value: ui_st.edit_action_value.clone(),
                    agent_name: ui_st.edit_agent_name.clone(),
                    profile_id: "default".to_string(),
                    enabled: ui_st.edit_enabled,
                    run_once: false,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                let name = task.name.clone();
                match ctx.task_store.create(task) {
                    Ok(()) => {
                        log::info!("Created scheduled task '{}'", name);
                        ctx.ui_state.scheduler_ui_state.creating_new = false;
                        ctx.ui_state.scheduler_ui_state.reset_editor();
                    }
                    Err(e) => log::error!("Failed to create scheduled task: {}", e),
                }
            }
            UiAction::SaveScheduledTask(tid) => {
                let ui_st = &ctx.ui_state.scheduler_ui_state;
                let task = crate::ai::scheduler::ScheduledTask {
                    id: tid.clone(),
                    name: ui_st.edit_name.clone(),
                    cron: ui_st.edit_cron.clone(),
                    action_type: ui_st.edit_action_type.clone(),
                    action_value: ui_st.edit_action_value.clone(),
                    agent_name: ui_st.edit_agent_name.clone(),
                    profile_id: "default".to_string(),
                    enabled: ui_st.edit_enabled,
                    run_once: false,
                    created_at: ctx
                        .task_store
                        .get(&tid)
                        .map(|t| t.created_at.clone())
                        .unwrap_or_default(),
                };
                if let Err(e) = ctx.task_store.update(&tid, task) {
                    log::error!("Failed to save scheduled task '{}': {}", tid, e);
                } else {
                    ctx.ui_state.scheduler_ui_state.selected_task_id = None;
                    ctx.ui_state.scheduler_ui_state.fields_loaded_for = None;
                }
            }
            UiAction::DeleteScheduledTask(tid) => {
                if let Err(e) = ctx.task_store.delete(&tid) {
                    log::error!("Failed to delete scheduled task '{}': {}", tid, e);
                }
            }
            UiAction::ToggleScheduledTask(tid) => match ctx.task_store.toggle_enabled(&tid) {
                Ok(new_state) => {
                    log::info!(
                        "Scheduled task '{}' is now {}",
                        tid,
                        if new_state { "enabled" } else { "disabled" }
                    );
                }
                Err(e) => log::error!("Failed to toggle scheduled task '{}': {}", tid, e),
            },
            _ => {}
        }
    }
}
