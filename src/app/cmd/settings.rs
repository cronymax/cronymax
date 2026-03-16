//! Window/profile/settings UI action handlers extracted from commands.rs

use crate::app::*;

pub(in crate::app) fn handle_ui_action_settings(
    state: &mut AppState,
    action: UiAction,
    #[allow(unused)] event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::PinTab(sid) if !state.ui_state.pinned_tabs.contains(&sid) => {
            state.ui_state.pinned_tabs.push(sid);
            // Re-activate the pinned pane so it stays focused instead
            // of having egui_tiles switch to the next visible tab.
            if state.webview_tabs.iter().any(|wt| wt.id == sid) {
                tiles::activate_browser_view_tab(&mut state.tile_tree, sid);
            } else {
                tiles::activate_terminal_tab(&mut state.tile_tree, sid);
            }
            log::info!("Pinned tab session {} (hidden from tab bar)", sid);
            state.scheduler.mark_dirty();
        }
        UiAction::UnpinTab(sid) => {
            state.ui_state.pinned_tabs.retain(|&s| s != sid);
            log::info!("Unpinned tab session {} (shown in tab bar)", sid);
            state.scheduler.mark_dirty();
        }
        UiAction::StartWindowDrag => {
            // Delegate to the OS for smooth, flicker-free window movement.
            let _ = state.window.drag_window();
        }
        UiAction::CloseWindow => {
            // Request exit — the event loop will handle cleanup.
            log::info!("CloseWindow action: requesting exit");
            // We store a flag; the event_loop.exit() call happens in the event handler.
            // For now, just close all sessions and drop.
            std::process::exit(0);
        }
        UiAction::Minimize => {
            state.window.set_minimized(true);
        }
        UiAction::ToggleMaximize => {
            let is_max = state.window.is_maximized();
            state.window.set_maximized(!is_max);
        }
        UiAction::PopOutOverlay => {
            // Promote the active webview to a floating overlay.
            //
            // If already an Overlay → promote to Independent (own egui browser).
            // If Docked → promote to Overlay:
            //   1. Create Modal (ModalPanel + wgpu surface + egui)
            //   2. Reparent wry WebView to the overlay panel
            //   3. Remove from tile tree, release tile space
            //   4. Update mode + z-order manager
            if let Some(idx) = state.ui_state.active_webview
                && let Some(tab) = state.webview_tabs.get_mut(idx)
            {
                let wid = tab.id;
                let url = tab.url.clone();

                match tab.mode {
                    BrowserViewMode::Overlay => {
                        // Already overlay → promote to independent.
                        state
                            .webview_manager
                            .promote_to_independent_with_url(wid, &url);
                        log::info!("Promoted overlay webview {} to independent", wid);
                    }
                    BrowserViewMode::Docked => {
                        // Docked → Overlay: create child panel and reparent.
                        let win_size = state.window.inner_size();
                        let scale = state.window.scale_factor() as f32;
                        let address_bar_h = state.styles.address_bar_height();

                        // Default overlay position: centered, 70% of window size.
                        let panel_lw = (win_size.width as f32 / scale) * 0.7;
                        let panel_lh = (win_size.height as f32 / scale) * 0.7;
                        let panel_lx = ((win_size.width as f32 / scale) - panel_lw) / 2.0;
                        let panel_ly = ((win_size.height as f32 / scale) - panel_lh) / 2.0;

                        match crate::renderer::panels::ModalPanel::new(
                            &state.window,
                            Some(event_loop),
                            panel_lx,
                            panel_ly,
                            panel_lw,
                            panel_lh,
                            scale,
                        ) {
                            Ok(panel) => {
                                // Reparent the wry WebView to the overlay panel.
                                tab.manager.repaint_webview(&state.window);

                                // Set webview bounds within the overlay panel
                                // (below the browser area).
                                let phys_w = (panel_lw * scale).round() as u32;
                                let total_phys_h = (panel_lh * scale).round() as u32;
                                let browser_phys_h = (address_bar_h * scale).round() as u32;
                                let wv_phys_h = total_phys_h.saturating_sub(browser_phys_h);
                                tab.manager.set_bounds(Bounds::new(
                                    0,
                                    browser_phys_h,
                                    phys_w,
                                    wv_phys_h,
                                ));
                                tab.manager.set_visible(true);

                                // Create Modal for browser rendering.
                                #[cfg(any(target_os = "macos", target_os = "windows"))]
                                match crate::ui::overlay::Modal::new(
                                    &state.gpu,
                                    panel,
                                    phys_w,
                                    total_phys_h,
                                    scale,
                                ) {
                                    Ok(overlay) => {
                                        tab.address_bar.url = url.clone();
                                        tab.manager.overlay = Some(overlay);
                                    }
                                    Err(e) => {
                                        log::warn!("Failed to create Modal: {e}");
                                    }
                                }

                                tab.mode = BrowserViewMode::Overlay;

                                // Remove from tile tree.
                                tiles::remove_browser_view_pane(&mut state.tile_tree, wid);

                                // Update z-order manager.
                                state.webview_manager.promote_to_overlay(wid);

                                log::info!("Promoted docked webview {} to overlay", wid);
                            }
                            Err(e) => {
                                log::warn!("Failed to create child panel for pop-out: {e}");
                            }
                        }

                        state.scheduler.mark_dirty();
                    }
                }
            }
        }
        UiAction::BringOverlayToFront(wid) => {
            state.webview_manager.bring_to_front(wid);
            // Also set it as the active webview.
            if let Some(idx) = state.webview_tabs.iter().position(|wt| wt.id == wid) {
                state.active_webview = idx;
                state.ui_state.active_webview = Some(idx);
            }
            // Dismiss any stale tooltip from the previously-frontmost overlay.
            state.float_panel_state.clear();
            log::info!("Brought overlay webview {} to front", wid);
        }
        UiAction::OpenSettings => {
            state.settings_state.open = true;
            state.scheduler.mark_dirty();
            log::info!("OpenSettings action received");
            // Refresh installed skills list for the panel.
            if let Some(ref sm) = state.skills_manager
                && let Ok(reg) = sm.load_registry()
            {
                state.skills_panel_state.installed_list = reg.skills.into_iter().collect();
            }
        }
        UiAction::CloseSettings => {
            state.settings_state.open = false;
            state.scheduler.mark_dirty();
            log::info!("CloseSettings action received");
        }
        UiAction::OpenOverlay => {
            let url = state
                .config
                .webview
                .default_url
                .clone()
                .unwrap_or_else(|| "https://google.com".to_string());
            log::info!("OpenOverlay action: opening {}", url);
            open_webview(state, &url, event_loop);
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
            if let Some(chat) = state.session_chats.get_mut(&session_id) {
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
            open_webview(state, &url, event_loop);
        }
        UiAction::CloseWebviewTab(wid) => {
            // Close the webview tab by ID.
            handle_ui_action(state, UiAction::CloseWebview(wid), event_loop);
        }
        UiAction::InstallAgent => {
            // Open native file dialog to pick agent directory.
            let dialog = rfd::FileDialog::new().set_title("Select Agent Directory");
            if let Some(path) = dialog.pick_folder() {
                match state.agent_registry.install(&path) {
                    Ok(name) => {
                        log::info!("Agent '{}' installed from {:?}", name, path);
                    }
                    Err(e) => {
                        log::error!("Failed to install agent: {}", e);
                    }
                }
            }
        }
        UiAction::UninstallAgent(name) => match state.agent_registry.uninstall(&name) {
            Ok(()) => {
                log::info!("Agent '{}' uninstalled", name);
            }
            Err(e) => {
                log::error!("Failed to uninstall agent '{}': {}", name, e);
            }
        },
        UiAction::ToggleAgent(name) => {
            let entry = state
                .agent_registry
                .installed
                .iter()
                .find(|a| a.name == name);
            let currently_enabled = entry.map(|e| e.enabled).unwrap_or(false);
            let result = if currently_enabled {
                state.agent_registry.disable(&name)
            } else {
                state.agent_registry.enable(&name)
            };
            if let Err(e) = result {
                log::error!("Failed to toggle agent '{}': {}", name, e);
            }
        }
        UiAction::CreateProfile => {
            let name = state.profiles_ui_state.edit_name.clone();
            let mut mgr = state.profile_manager.lock().unwrap();
            match mgr.create_profile(&name) {
                Ok(p) => {
                    let new_id = p.id.clone();
                    log::info!("Created profile '{}'", new_id);
                    state.profiles_ui_state.selected_profile_id = Some(new_id);
                    state.profiles_ui_state.creating_new = false;
                }
                Err(e) => log::error!("Failed to create profile: {}", e),
            }
        }
        UiAction::SaveProfile(pid) => {
            let name = state.profiles_ui_state.edit_name.clone();
            let sandbox = state.profiles_ui_state.sandbox_policy_from_state();
            let mut mgr = state.profile_manager.lock().unwrap();
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
            state.profiles_ui_state.save_status =
                Some((msg.into(), state.egui.ctx.input(|i| i.time)));

            // Detect if sandbox rules changed from what was applied at PTY spawn.
            // If they differ, show a relaunch dialog so the OS-level sandbox is re-applied.
            if ok && sandbox != state.applied_sandbox_policy {
                state.profiles_ui_state.show_relaunch_dialog = true;
                log::info!("Sandbox policy changed — relaunch required for OS-level enforcement");
            }
        }
        UiAction::DuplicateProfile(pid) => {
            let new_name = format!("{} (copy)", state.profiles_ui_state.edit_name);
            let mut mgr = state.profile_manager.lock().unwrap();
            match mgr.duplicate_profile(&pid, &new_name) {
                Ok(new_id) => {
                    log::info!("Duplicated profile '{}' → '{}'", pid, new_id);
                    state.profiles_ui_state.selected_profile_id = Some(new_id);
                    state.profiles_ui_state.fields_loaded_for = None;
                }
                Err(e) => log::error!("Failed to duplicate profile '{}': {}", pid, e),
            }
        }
        UiAction::DeleteProfile(pid) => {
            let mut mgr = state.profile_manager.lock().unwrap();
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
                state.profiles_ui_state.save_status = Some((
                    format!("Delete failed: {}", e),
                    state.egui.ctx.input(|i| i.time),
                ));
            }
        }
        UiAction::SetActiveProfile(pid) => {
            let mut mgr = state.profile_manager.lock().unwrap();
            // Load the new profile's sandbox policy *before* switching.
            let new_sandbox = mgr
                .sandbox_policy(&pid)
                .unwrap_or_else(|_| crate::profile::sandbox::policy::SandboxPolicy::from_default());
            match mgr.set_active(&pid) {
                Ok(()) => {
                    log::info!("Switched active profile to '{}'", pid);
                    drop(mgr); // release lock before mutating ui state
                    // If the sandbox policy differs from the one applied at process
                    // launch, prompt the user to relaunch.
                    if new_sandbox != state.applied_sandbox_policy {
                        state.ui_state.show_profile_relaunch_dialog = true;
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
            let providers = state.providers_ui_state.to_provider_configs();
            match crate::config::save_providers_to_config(&providers) {
                Ok(()) => {
                    log::info!("Saved {} LLM providers to config.toml", providers.len());
                    // Update the in-memory config too.
                    if state.config.ai.is_none() {
                        state.config.ai = Some(crate::config::AiConfig::default());
                    }
                    if let Some(ref mut ai) = state.config.ai {
                        ai.providers = Some(providers.clone());
                    }
                    // Feed updated providers to the LLM client.
                    if let Some(ref mut client) = state.llm_client {
                        client.set_configured_providers(providers);
                        // Re-fetch available models with the new provider config.
                        client.fetch_available_models(state.proxy.clone(), &state.runtime);
                    }
                    state.providers_ui_state.save_status =
                        Some(("Saved!".into(), state.egui.ctx.input(|i| i.time)));
                }
                Err(e) => {
                    log::error!("Failed to save providers: {}", e);
                    state.providers_ui_state.save_status =
                        Some((format!("Error: {}", e), state.egui.ctx.input(|i| i.time)));
                }
            }
        }
        UiAction::CreateScheduledTask
        | UiAction::SaveScheduledTask(_)
        | UiAction::DeleteScheduledTask(_)
        | UiAction::ToggleScheduledTask(_) => {
            super::scheduler::handle_scheduled_task_action(state, action);
        }
        _ => {}
    }
}
