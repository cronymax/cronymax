//! The `Ui` model — owns `FrameWindow` and all UI-layer state.
//!
//! `app::AppState` keeps a single `pub(crate) ui: Ui` field; the rest of
//! `AppState` is lifecycle/daemon logic.  `Ui` provides convenience methods
//! that wrap the underlying fields so call-sites in `app/` stay concise.
//!
//! Dependencies that `Ui` does **not** own (terminal sessions, LLM client,
//! config, …) are passed via [`AppCtx`] — a bag of mutable references to
//! all non-UI `AppState` fields that the UI dispatch handlers need.

use std::collections::HashMap;
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;

use crate::ai::stream::AppEvent;
use crate::config::AppConfig;
use crate::renderer::atlas::CellSize;
use crate::renderer::terminal::{SessionId, TerminalSession};
use crate::ui::UiState;

// ─── AppCtx — non-UI dependencies injected from AppState ─────────────────────

/// Non-UI dependencies that [`Ui`] dispatch methods need from [`AppState`].
///
/// Created via [`AppState::split_ui()`] which splits the borrow so `&mut Ui`
/// and `&mut AppCtx` can coexist.
pub(crate) struct AppCtx<'a> {
    pub ui_state: &'a mut UiState,
    // ── Lifecycle / config ─────────────────────────────────────────────
    pub config: &'a mut AppConfig,
    pub cell_size: CellSize,
    pub sessions: &'a mut HashMap<SessionId, TerminalSession>,
    pub next_id: &'a mut SessionId,
    pub scheduler: &'a mut crate::renderer::scheduler::FrameScheduler,

    // ── AI / LLM integration ─────────────────────────────────────────
    pub runtime: &'a Arc<tokio::runtime::Runtime>,
    pub proxy: &'a EventLoopProxy<AppEvent>,
    pub session_chats: &'a mut HashMap<SessionId, crate::ui::chat::SessionChat>,
    pub llm_client: &'a mut Option<crate::ai::client::LlmClient>,
    pub token_counter: &'a mut crate::ai::context::TokenCounter,
    pub skill_registry: &'a mut crate::ai::skills::SkillRegistry,
    pub profile_manager: &'a Arc<std::sync::Mutex<crate::profile::ProfileManager>>,
    pub agent_registry: &'a mut crate::ai::agent::AgentRegistry,
    pub skills_manager: &'a Option<crate::ai::skills::manager::SkillsManager>,
    pub loaded_external_skills: &'a mut Vec<crate::ai::skills::loader::ExternalSkill>,

    // ── Persistence / secrets ────────────────────────────────────────
    pub db_store: &'a Option<crate::ai::db::DbStore>,
    pub secret_store: &'a Arc<crate::services::secret::SecretStore>,

    // ── Sandbox ──────────────────────────────────────────────────────
    pub applied_sandbox_policy: &'a crate::profile::sandbox::policy::SandboxPolicy,

    // ── Agent execution ──────────────────────────────────────────────
    pub pending_results: &'a mut crate::ai::stream::PendingResultMap,

    // ── Scheduler ────────────────────────────────────────────────────
    pub task_store: &'a mut crate::ai::scheduler::ScheduledTaskStore,
    pub scheduler_history_cache: &'a [crate::ai::scheduler::ExecutionRecord],

    // ── Channel subsystem ────────────────────────────────────────────
    pub channel_manager: &'a mut Option<crate::channels::ChannelManager>,
    pub channel_messages: &'a HashMap<String, Vec<crate::channels::ChannelDisplayMessage>>,

    // ── Frame / render bookkeeping ───────────────────────────────────
    pub frame_count: &'a mut u64,
    pub prev_grid_sizes: &'a mut HashMap<SessionId, (u16, u16)>,

    pub cursor_visible: bool,
}

impl AppCtx<'_> {
    /// Retrieve the active profile's sandbox policy, falling back to the default.
    pub fn active_sandbox_policy(&self) -> crate::profile::sandbox::policy::SandboxPolicy {
        let mgr = self.profile_manager.lock().unwrap();
        mgr.active()
            .and_then(|p| p.sandbox.clone())
            .unwrap_or_else(crate::profile::sandbox::policy::SandboxPolicy::from_default)
    }
}
