//! Application lifecycle handlers extracted from app/mod.rs

mod init;
mod llm;

use super::*;

pub(super) fn handle_resumed(app: &mut App, event_loop: &ActiveEventLoop) {
    if app.state.is_some() {
        return;
    }

    let window_attrs = Window::default_attributes()
        .with_title("cronymax")
        .with_inner_size(LogicalSize::new(992, 768))
        .with_transparent(true)
        .with_blur(true);

    // Platform-specific: custom titlebar configuration.
    #[cfg(target_os = "macos")]
    let window_attrs = {
        use winit::platform::macos::WindowAttributesExtMacOS;
        window_attrs
            .with_titlebar_hidden(true)
            .with_fullsize_content_view(true)
    };
    #[cfg(target_os = "linux")]
    let window_attrs = window_attrs.with_decorations(false);
    #[cfg(target_os = "windows")]
    let window_attrs = {
        use winit::platform::windows::WindowAttributesExtWindows;
        window_attrs
            .with_decorations(false)
            .with_undecorated_shadow(true)
    };

    let window = Arc::new(
        event_loop
            .create_window(window_attrs)
            .expect("Failed to create window"),
    );

    let gpu = pollster::block_on(GpuContext::new(window.clone()));
    log::info!(
        "GPU context initialized, alpha_mode={:?}",
        gpu.surface_config.alpha_mode
    );

    // macOS: configure window appearance (shadow, transparency, etc.).
    // Must run AFTER GPU context so wgpu's surface doesn't override settings.
    #[cfg(target_os = "macos")]
    crate::renderer::platform::macos::setup_window_appearance(&window);

    // Initialize the text renderer.
    let format = gpu.surface_config.format;
    let renderer = TerminalRenderer::new(
        &gpu.device,
        &gpu.queue,
        format,
        &app.config.font.family,
        app.config.font.size,
        app.config.font.line_height,
    );

    // Compute initial viewport and grid size.
    let size = window.inner_size();
    // Apply system dark/light palette before first use.
    let styles = app.config.styles.clone();
    let (viewport, cols, rows) =
        ui::compute_single_pane(size.width, size.height, &renderer.cell_size, &styles);

    // Initialize egui integration.
    let egui = EguiIntegration::new(&window, &gpu.device, gpu.surface_config.format);
    egui.ctx.set_style(app.config.resolve_egui_style());

    // Bridge egui repaint requests to the winit event loop proxy.
    {
        let proxy = app.proxy.clone().expect("EventLoopProxy not set");
        egui.ctx.set_request_repaint_callback(move |info| {
            // Route ALL repaint requests through the timer path so that
            // egui's zero-delay requests (cursor blink, hover animations)
            // don't spin the CPU at max frame rate.  Clamp the minimum
            // delay to 16 ms (~60 fps) — fast enough for smooth UI
            // interaction while keeping idle CPU near zero.
            let delay = info.delay.max(std::time::Duration::from_millis(16));
            let _ = proxy.send_event(AppEvent::RequestRepaintAfter { delay });
        });
    }

    // Determine shell.
    let shell = app
        .config
        .terminal
        .shell
        .clone()
        .unwrap_or_else(crate::renderer::platform::default_shell);

    // Load profile manager early so we can apply sandbox to the first PTY.
    let profile_manager = Arc::new(std::sync::Mutex::new(
        crate::profile::ProfileManager::load(
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("cronymax")
                .join("profiles"),
        )
        .unwrap_or_else(|e| {
            log::warn!("Failed to load profiles: {}. Using defaults.", e);
            let base_dir = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("cronymax")
                .join("profiles");
            let _ = std::fs::create_dir_all(&base_dir);
            crate::profile::ProfileManager::load(base_dir)
                .expect("Failed to create profile manager")
        }),
    ));

    // Apply --profile CLI override (e.g. "New Window with Profile").
    if let Some(ref pid) = app.profile_override {
        let mut mgr = profile_manager.lock().unwrap();
        match mgr.set_active(pid) {
            Ok(()) => log::info!("Applied --profile override: '{}'", pid),
            Err(e) => log::warn!("--profile '{}' not found, using default: {}", pid, e),
        }
    }

    // Get sandbox policy from the active profile for PTY confinement.
    let sandbox_policy = {
        let mgr = profile_manager.lock().unwrap();
        mgr.active()
            .and_then(|p| p.sandbox.clone())
            .unwrap_or_else(crate::sandbox::policy::SandboxPolicy::from_default)
    };

    // Spawn terminal session (sandboxed).
    let session = TerminalSession::new(
        1,
        &shell,
        cols,
        rows,
        app.config.terminal.scrollback_lines,
        Some(&sandbox_policy),
        app.proxy.clone(),
    );
    log::info!(
        "Terminal session started: {}x{} shell={} sandboxed=true",
        cols,
        rows,
        shell
    );

    let mut sessions = HashMap::new();
    sessions.insert(1, session);
    let tile_tree = tiles::create_initial_terminal_tree(1, "cronymax");

    // Per-session prompt editors.
    let mut prompt_editors = HashMap::new();
    let mut prompt_editor = crate::ui::prompt::PromptState::new();
    prompt_editor.visible = false;
    prompt_editors.insert(1_u32, prompt_editor);

    // Initial UiState synced from sessions.
    let mut ui_state = UiState::new();
    ui_state.claw_enabled = app.config.claw.as_ref().is_some_and(|c| c.enabled);
    ui_state.tabs.push(TabInfo::Terminal {
        session_id: 1,
        title: "cronymax".into(),
    });

    let shared_secret_store = Arc::new(crate::secret::SecretStore::default());

    app.state = Some(AppState {
        window,
        gpu,
        config: app.config.clone(),
        renderer,
        sessions,
        tile_tree,
        tile_rects: Vec::new(),
        prev_grid_sizes: {
            let mut m = HashMap::new();
            m.insert(1, (cols, rows));
            m
        },
        next_id: 2,
        viewport,
        modifiers: ModifiersState::empty(),
        webview_tabs: Vec::new(),
        active_webview: 0,
        next_webview_id: 1,
        webview_manager: WebviewManager::default(),
        float_panel_state: FloatPanelState::default(),
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        float_renderer: None,
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        settings_overlay: None,
        split: None,
        colon_buf: None,
        text_scratch: String::with_capacity(80 * 24 + 24),
        frame_count: 0,
        mouse_x: 0.0,
        mouse_y: 0.0,
        hovered_link: None,
        ime_composing: false,
        ime_enabled: false,
        egui,
        ui_state,
        styles,
        prompt_editors,
        pane_widgets: tiles::PaneWidgetStore::default(),
        runtime: app.runtime.clone(),
        proxy: app.proxy.clone().expect("EventLoopProxy not set"),
        session_chats: HashMap::new(),
        llm_session_map: HashMap::new(),
        llm_client: None,
        token_counter: crate::ai::context::TokenCounter::new(),
        skill_registry: crate::ai::skills::SkillRegistry::new(),
        settings_state: crate::ui::settings::SettingsState::default(),
        general_ui_state: {
            let claw_enabled = app.config.claw.as_ref().is_some_and(|c| c.enabled);
            crate::ui::settings::general::GeneralSettingsState::new(claw_enabled)
        },
        agent_registry: {
            let mut reg = crate::ai::agent::AgentRegistry::default_dir();
            if let Err(e) = reg.load() {
                log::warn!("Failed to load agent registry: {}", e);
            }
            reg
        },
        agents_ui_state: crate::ui::settings::agents::AgentsSettingsState::default(),
        skills_manager: None, // initialized after config is fully loaded
        loaded_external_skills: Vec::new(),
        skills_panel_state: crate::ui::skills_panel::SkillsPanelState::new(),
        profiles_ui_state: crate::ui::settings::profiles::ProfilesSettingsState::default(),
        providers_ui_state:
            crate::ui::settings::providers::ProvidersSettingsState::with_secret_store(
                shared_secret_store.clone(),
            ),
        task_store: {
            let mut ts = crate::ai::scheduler::ScheduledTaskStore::new();
            if let Err(e) = ts.load() {
                log::warn!("Failed to load scheduled tasks: {}", e);
            }
            ts
        },
        scheduler_ui_state: crate::ui::settings::scheduler::SchedulerSettingsState::default(),
        scheduler_history_cache: Vec::new(),
        db_store: None,
        budget_tracker: None,
        channel_manager: None,
        channels_ui_state:
            crate::ui::settings::channels::ChannelsSettingsState::from_claw_config_with_store(
                app.config.claw.as_ref(),
                shared_secret_store.clone(),
            ),
        onboarding_wizard_state: None,
        pending_channel_replies: HashMap::new(),
        _messages_received_channel_counter: 0,
        channel_messages: HashMap::new(),
        secret_store: shared_secret_store,
        applied_sandbox_policy: sandbox_policy.clone(),
        pending_results: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pending_terminal_execs: Vec::new(),
        shared_tab_info: Arc::new(std::sync::Mutex::new(Vec::new())),
        shared_terminal_info: Arc::new(std::sync::Mutex::new(Vec::new())),
        shared_onboarding_state: Arc::new(std::sync::Mutex::new(None)),
        scheduler: {
            let mut s = crate::renderer::scheduler::FrameScheduler::new(
                std::time::Duration::from_millis(8),
            );
            crate::renderer::scheduler::RenderSchedule::mark_dirty(&mut s);
            s
        },
        next_cursor_blink: Some(std::time::Instant::now() + std::time::Duration::from_millis(530)),
        cursor_visible: true,
        profile_manager,
    });

    // Post-initialization: set up LLM client and skills from active profile.
    if let Some(state) = app.state.as_mut() {
        // Build LlmConfig from config.toml [ai] section + first configured provider.
        let ai_cfg = state.config.ai.clone().unwrap_or_default();
        let first_provider = ai_cfg.providers.as_ref().and_then(|p| p.first().cloned());

        let (provider, api_base, api_key_env) = if let Some(ref pc) = first_provider {
            let prov = match pc.provider_type.as_str() {
                "ollama" => crate::ai::client::LlmProvider::Ollama,
                "copilot" | "github-copilot" => crate::ai::client::LlmProvider::Copilot,
                "anthropic" | "claude" => crate::ai::client::LlmProvider::Anthropic,
                "custom" => crate::ai::client::LlmProvider::Custom,
                _ => crate::ai::client::LlmProvider::OpenAI,
            };
            (prov, pc.api_base.clone(), pc.api_key_env.clone())
        } else {
            // Fallback: auto-detect from keychain + env vars.
            llm::detect_provider(&state.secret_store)
        };

        let secret_storage = first_provider
            .as_ref()
            .map(|pc| pc.secret_storage.clone())
            .unwrap_or_default();

        let llm_config = crate::ai::client::LlmConfig {
            provider,
            model: "gpt-4o".into(),
            api_base,
            api_key_env,
            max_context_tokens: ai_cfg.max_context_tokens.unwrap_or(128_000),
            reserve_tokens: ai_cfg.reserve_tokens.unwrap_or(4096),
            system_prompt: ai_cfg.system_prompt.or_else(|| {
                Some(
                    crate::ai::client::LlmConfig::default()
                        .system_prompt
                        .unwrap_or_default(),
                )
            }),
            auto_compact: ai_cfg.auto_compact.unwrap_or(false),
            secret_storage,
        };

        // Create LLM client and configure providers.
        let mut client = crate::ai::client::LlmClient::new(&llm_config, &state.secret_store);
        if let Some(ref providers) = ai_cfg.providers {
            client.set_configured_providers(providers.clone());
        }
        // If no providers in config, auto-detect from keychain + env vars
        // so fetch_available_models uses correct endpoints.
        if client.configured_providers().is_empty() {
            let auto_store = &state.secret_store;
            let mut auto = Vec::new();
            let has_key = |provider: &str, env: &str| {
                auto_store
                    .resolve(
                        &crate::secret::provider_api_key(provider),
                        Some(env),
                        &crate::secret::SecretStorage::Auto,
                    )
                    .ok()
                    .flatten()
                    .is_some()
            };
            if has_key("openai", "OPENAI_API_KEY") {
                auto.push(crate::config::ProviderConfig {
                    name: "OpenAI".into(),
                    provider_type: "openai".into(),
                    api_base: Some("https://api.openai.com/v1".into()),
                    api_key_env: Some("OPENAI_API_KEY".into()),
                    secret_storage: Default::default(),
                });
            }
            if has_key("copilot", "GH_TOKEN") {
                auto.push(crate::config::ProviderConfig {
                    name: "GitHub Copilot".into(),
                    provider_type: "copilot".into(),
                    api_base: Some("https://models.inference.ai.azure.com".into()),
                    api_key_env: Some("GH_TOKEN".into()),
                    secret_storage: Default::default(),
                });
            }
            if has_key("anthropic", "ANTHROPIC_API_KEY") {
                auto.push(crate::config::ProviderConfig {
                    name: "Anthropic".into(),
                    provider_type: "anthropic".into(),
                    api_base: Some("https://api.anthropic.com/v1".into()),
                    api_key_env: Some("ANTHROPIC_API_KEY".into()),
                    secret_storage: Default::default(),
                });
            }
            // Always add Ollama — it's local and needs no key.
            auto.push(crate::config::ProviderConfig {
                name: "Ollama".into(),
                provider_type: "ollama".into(),
                api_base: Some("http://localhost:11434/v1".into()),
                api_key_env: None,
                secret_storage: Default::default(),
            });
            if !auto.is_empty() {
                log::info!(
                    "Auto-detected {} provider(s) for model fetching",
                    auto.len()
                );
                client.set_configured_providers(auto);
            }
        }
        state.llm_client = Some(client);

        // Seed prompt editors with the current model, then fetch full lists from APIs.
        if let Some(ref client) = state.llm_client {
            let seed = vec![client.current_model_item()];
            for pe in state.prompt_editors.values_mut() {
                pe.model_items = seed.clone();
            }
            client.fetch_available_models(state.proxy.clone(), &state.runtime);
        }

        // Initialize the chat state for the first session (id=1).
        let mut chat = crate::ui::chat::SessionChat::new(
            llm_config.max_context_tokens,
            llm_config.reserve_tokens,
        );
        if let Some(ref sp) = llm_config.system_prompt {
            // Append channel context so the LLM knows about configured integrations.
            let full_prompt =
                if let Some(ctx) = crate::app::chat::build_channel_context(&state.config) {
                    format!("{}\n\n{}", sp, ctx)
                } else {
                    sp.clone()
                };
            chat.set_system_prompt(&full_prompt, &state.token_counter, &llm_config.model);
        }
        state.session_chats.insert(1, chat);

        log::info!(
            "AI initialized: provider={:?}, model={}",
            llm_config.provider,
            llm_config.model
        );

        // ── On-launch session restore (T036) ──────────────────────────
        // Try to restore the saved layout, chat histories, and command
        // history from the active profile directory.
        {
            let mgr = state.profile_manager.lock().unwrap();
            let profile_dir = mgr
                .active()
                .map(|p| mgr.profile_dir(&p.id))
                .unwrap_or_else(|| mgr.profile_dir("default"));
            drop(mgr);

            if let Ok(snapshot) = crate::app::session_persist::load_layout(&profile_dir) {
                let mut next_id = state.next_id;
                let (tree, restored_tabs) =
                    crate::app::session_persist::reconstruct_tree(&snapshot, &mut next_id);

                // Replace default tile tree.
                state.tile_tree = tree;
                // Clear the defaults created above.
                state.sessions.clear();
                state.prompt_editors.clear();
                state.session_chats.clear();
                state.ui_state.tabs.clear();

                let shell = state
                    .config
                    .terminal
                    .shell
                    .clone()
                    .unwrap_or_else(crate::renderer::platform::default_shell);
                let (_, cols, rows) = crate::ui::compute_single_pane(
                    state.window.inner_size().width,
                    state.window.inner_size().height,
                    &state.renderer.cell_size,
                    &state.styles,
                );
                let sandbox = {
                    let mgr = state.profile_manager.lock().unwrap();
                    mgr.active()
                        .and_then(|p| p.sandbox.clone())
                        .unwrap_or_else(crate::sandbox::policy::SandboxPolicy::from_default)
                };

                for tab in &restored_tabs {
                    // Create PTY session for Chat and Terminal tabs.
                    if matches!(
                        tab.tab_type,
                        crate::app::session_persist::TabType::Chat
                            | crate::app::session_persist::TabType::Terminal
                    ) {
                        let session = TerminalSession::new(
                            tab.session_id,
                            &shell,
                            cols,
                            rows,
                            state.config.terminal.scrollback_lines,
                            Some(&sandbox),
                            Some(state.proxy.clone()),
                        );
                        state.sessions.insert(tab.session_id, session);
                    }

                    // Create prompt editor.
                    let mut pe = crate::ui::prompt::PromptState::new();
                    pe.visible = true;
                    if let Some(existing) = state.prompt_editors.values().next() {
                        pe.model_items = existing.model_items.clone();
                        pe.selected_model_idx = existing.selected_model_idx;
                    }
                    state.prompt_editors.insert(tab.session_id, pe);

                    // Create session chat and restore history if available.
                    let mut chat = crate::ui::chat::SessionChat::new(
                        llm_config.max_context_tokens,
                        llm_config.reserve_tokens,
                    );
                    chat.persistent_id = Some(tab.persistent_id.clone());
                    if let Some(ref sp) = llm_config.system_prompt {
                        chat.set_system_prompt(sp, &state.token_counter, &llm_config.model);
                    }

                    // Restore model override from snapshot (T037).
                    if let Some(ref ms) = tab.model_selection {
                        let provider = match ms.provider.as_str() {
                            "Ollama" => crate::ai::client::LlmProvider::Ollama,
                            "Copilot" => crate::ai::client::LlmProvider::Copilot,
                            "Anthropic" => crate::ai::client::LlmProvider::Anthropic,
                            "Custom" => crate::ai::client::LlmProvider::Custom,
                            _ => crate::ai::client::LlmProvider::OpenAI,
                        };
                        chat.model_override = Some(crate::ai::client::ModelSelection {
                            provider,
                            model: ms.model.clone(),
                            display_label: ms.model.clone(),
                        });
                    }

                    // Load saved chat history.
                    if tab.tab_type == crate::app::session_persist::TabType::Chat {
                        match crate::app::session_persist::load_session_file(
                            &tab.persistent_id,
                            &profile_dir,
                        ) {
                            Ok(record) => {
                                chat.messages = record.messages.clone();
                                for msg in &record.messages {
                                    chat.history.push(msg.clone());
                                }
                                chat.tokens_used = record.token_count;
                                log::info!(
                                    "Restored session {} ({} messages)",
                                    tab.persistent_id,
                                    record.messages.len()
                                );
                            }
                            Err(e) => {
                                // T038: Corrupted or missing session file — skip and notify.
                                log::warn!("Failed to load session {}: {}", tab.persistent_id, e);
                                chat.add_info_message(&format!(
                                    "Could not restore previous chat history: {}",
                                    e
                                ));
                            }
                        }
                    }

                    state.session_chats.insert(tab.session_id, chat);

                    // Populate ui_state.tabs.
                    let tab_info = match tab.tab_type {
                        crate::app::session_persist::TabType::Chat => TabInfo::Chat {
                            session_id: tab.session_id,
                            title: tab.title.clone(),
                        },
                        crate::app::session_persist::TabType::Terminal => TabInfo::Terminal {
                            session_id: tab.session_id,
                            title: tab.title.clone(),
                        },
                        _ => TabInfo::Chat {
                            session_id: tab.session_id,
                            title: tab.title.clone(),
                        },
                    };
                    state.ui_state.tabs.push(tab_info);
                }

                state.next_id = next_id;
                if snapshot.active_tab_index < state.ui_state.tabs.len() {
                    state.ui_state.active_tab = snapshot.active_tab_index;
                }

                // Restore command history into prompt editors.
                if let Ok(history) = crate::app::session_persist::load_command_history(&profile_dir)
                {
                    let cmds: Vec<String> = history.into_iter().map(|e| e.command).collect();
                    for pe in state.prompt_editors.values_mut() {
                        pe.history = cmds.clone();
                    }
                    log::info!("Restored {} command history entries", cmds.len());
                }

                log::info!(
                    "Session restore complete: {} tabs restored",
                    restored_tabs.len()
                );
            } else {
                log::info!("No saved layout found, using default single-tab layout");
            }
        }

        // Register built-in skills.
        state.skill_registry.register_builtins();
        state
            .skill_registry
            .register_memory_skills(state.profile_manager.clone());

        // Initialize SQLite persistence store and budget tracker (before UI skills
        // so that chat skills can receive the db handle for persistent memory).
        let mut chat_db: Option<crate::ai::db::DbStore> = None;
        let mut chat_profile_id = "default".to_string();
        match crate::ai::db::DbStore::open_default() {
            Ok(db) => {
                state.db_store = Some(db.clone());

                // Get active profile ID for budget scoping.
                let profile_id = {
                    let mgr = state.profile_manager.lock().unwrap();
                    mgr.active()
                        .map(|p| p.id.clone())
                        .unwrap_or_else(|| "default".to_string())
                };

                // Create budget tracker with default limits.
                let budget = crate::ai::budget::BudgetTracker::new(
                    db.clone(),
                    profile_id.clone(),
                    crate::ai::budget::BudgetLimits::default(),
                );
                let budget_arc = Arc::new(std::sync::Mutex::new(budget));
                state.budget_tracker = Some(budget_arc.clone());

                // Register sandbox skills (budget, audit).
                crate::ai::skills::sandbox::register_sandbox_skills(
                    &mut state.skill_registry,
                    db.clone(),
                    budget_arc,
                );

                chat_db = Some(db.clone());
                chat_profile_id = profile_id;

                log::info!("SQLite persistence initialized (memory FTS5, audit, budget)");

                // Migrate any plaintext OAuth tokens to system keychain.
                match db.migrate_oauth_token_to_keychain(&state.secret_store) {
                    Ok(0) => {}
                    Ok(n) => log::info!("Migrated {} OAuth token(s) to system keychain", n),
                    Err(e) => log::warn!("OAuth token migration failed: {}", e),
                }
            }
            Err(e) => {
                log::warn!("Failed to open SQLite store: {}. Persistence disabled.", e);
            }
        }

        // Register UI skills (webview, terminal, tab, browser, general, onboarding, chat).
        let webview_info = Arc::new(std::sync::Mutex::new(Vec::new()));
        let skill_deps = crate::ai::skills::SkillDependencies {
            proxy: state.proxy.clone(),
            pending_results: state.pending_results.clone(),
            webview_info,
            tab_info: state.shared_tab_info.clone(),
            terminal_info: state.shared_terminal_info.clone(),
            onboarding_state: state.shared_onboarding_state.clone(),
            db: chat_db,
            profile_id: chat_profile_id,
            secret_store: Some(state.secret_store.clone()),
        };
        state.skill_registry.register_ui_skills(&skill_deps);

        init::init_skills_and_channels(state);
    }
}
