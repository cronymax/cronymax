//! Skills and channel initialization extracted from lifecycle.rs

use crate::app::*;

pub(super) fn init_skills_and_channels(state: &mut AppState) {
    // ── Load external skills (OpenClaw) ──────────────────────────────
    {
        let skills_dir = state
            .config
            .skills
            .as_ref()
            .and_then(|s| s.skills_dir.as_deref())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::renderer::platform::config_dir().join("skills"));
        let api_base = state
            .config
            .skills
            .as_ref()
            .and_then(|s| s.clawhub_api_base.clone())
            .unwrap_or_else(|| "https://clawhub.ai".to_string());

        let sm = crate::ai::skills::manager::SkillsManager::new(skills_dir.clone(), api_base);

        // Load external skills from filesystem.
        let loader = crate::ai::skills::loader::SkillLoader::new(skills_dir);
        match loader.load_all(&state.config) {
            Ok(skills) => {
                let count = skills.len();
                crate::ai::skills::loader::register_external_skills(
                    &mut state.skill_registry,
                    &skills,
                );
                state.loaded_external_skills = skills;
                log::info!("Loaded {} external skills from filesystem", count);
            }
            Err(e) => {
                log::warn!("Failed to load external skills: {}", e);
            }
        }

        state.skills_manager = Some(sm);
    }

    // ── Start scheduler polling loop ─────────────────────────────────
    {
        let mut store = crate::ai::scheduler::ScheduledTaskStore::new();
        if let Err(e) = store.load() {
            log::warn!("Failed to load scheduled tasks for scheduler loop: {}", e);
        }
        let proxy = state.proxy.clone();
        let runtime = state.runtime.clone();
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        crate::ai::scheduler::start_scheduler_loop(&runtime, store, proxy, 30, running);
        log::info!("Scheduler polling loop started (30s interval)");
    }

    // ── Auto-start channels if claw.enabled and channels are configured ──
    let claw_enabled = state.config.claw.as_ref().is_some_and(|c| c.enabled);
    let has_channels = state
        .config
        .claw
        .as_ref()
        .map(|c| !c.channels.is_empty())
        .unwrap_or(false);

    if claw_enabled && has_channels {
        log::info!(
            "Claw mode enabled with channels configured — auto-starting channel registration"
        );
        let claw_config = state.config.claw.clone().unwrap_or_default();
        let proxy = state.proxy.clone();
        let runtime = state.runtime.clone();
        let ss = state.secret_store.clone();
        runtime.spawn(async move {
            let mut mgr = crate::channels::ChannelManager::new(proxy);
            if let Err(e) = crate::channels::register_channels(&mut mgr, &claw_config, ss).await {
                log::error!("Failed to auto-register channels on startup: {}", e);
            } else {
                log::info!("Channels auto-registered on startup");
            }
            // Keep the manager alive so the WS loop keeps running.
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    } else if claw_enabled {
        log::info!("Claw mode enabled but no channels configured — skipping auto-start");
    }
}
