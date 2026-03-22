//! Application state types extracted from app/mod.rs

use super::*;

/// Context for a pending terminal command execution with marker-based output capture.
pub(crate) struct PendingTerminalExec {
    pub(crate) marker: String,
    pub(crate) terminal_id: usize,
    /// Absolute row where we started (to capture output between start and marker).
    pub(crate) start_abs_row: i32,
    pub(crate) started_at: std::time::Instant,
    pub(crate) timeout_ms: u64,
    /// The full command string written to the PTY (used to estimate echo lines).
    pub(crate) full_cmd: String,
}

pub(crate) use crate::ui::actions::KeyAction as Action;

/// Application state — holds the window, GPU context, and all subsystems.
pub(crate) struct AppState {
    // ── UI model (frame + all view-layer state) ───────────────────────────
    /// The UI model owns `FrameWindow`, theming, layout, overlays, and
    /// all widget state.  `app/` accesses it via `state.ui.*`.
    pub(crate) ui: crate::ui::Ui,
    pub(crate) ui_state: crate::ui::UiState,

    // ── Lifecycle / config ───────────────────────────────────────────────
    pub(crate) config: AppConfig,

    /// All terminal sessions by ID.
    pub(crate) sessions: HashMap<SessionId, TerminalSession>,
    /// Previous grid dimensions per session (cols, rows) for PTY resize detection.
    pub(crate) prev_grid_sizes: HashMap<SessionId, (u16, u16)>,
    /// Next session ID counter.
    pub(crate) next_id: SessionId,
    /// Frame counter for diagnostics.
    pub(crate) frame_count: u64,

    // ── AI / LLM integration ─────────────────────────────────────────────
    /// Tokio runtime for async LLM/IO tasks.
    pub(crate) runtime: Arc<tokio::runtime::Runtime>,
    /// Event loop proxy for sending AppEvents from background tasks.
    pub(crate) proxy: EventLoopProxy<AppEvent>,
    /// Per-session chat state (messages, history, streaming handles).
    pub(crate) session_chats: HashMap<SessionId, crate::ui::chat::SessionChat>,
    /// Mapping from LLM stream session_id → terminal SessionId (for event routing).
    pub(crate) llm_session_map: HashMap<u32, SessionId>,
    /// LLM client for streaming completions.
    pub(crate) llm_client: Option<crate::ai::client::LlmClient>,
    /// Token counter for estimating token usage.
    pub(crate) token_counter: crate::ai::context::TokenCounter,
    /// Skills registry with built-in tools.
    pub(crate) skill_registry: crate::ai::skills::SkillRegistry,
    /// Profile manager (shared with skill handlers via Arc<Mutex>).
    pub(crate) profile_manager: Arc<std::sync::Mutex<crate::profile::ProfileManager>>,
    /// Agent registry for installable agent packages.
    pub(crate) agent_registry: crate::ai::agent::AgentRegistry,
    /// Skills manager for OpenClaw external skills lifecycle.
    pub(crate) skills_manager: Option<crate::ai::skills::manager::SkillsManager>,
    /// Loaded external skills (cached for system prompt injection).
    pub(crate) loaded_external_skills: Vec<crate::ai::skills::loader::ExternalSkill>,
    /// Scheduled task store.
    pub(crate) task_store: crate::ai::scheduler::ScheduledTaskStore,
    /// Cached execution history for the scheduler UI.
    pub(crate) scheduler_history_cache: Vec<crate::ai::scheduler::ExecutionRecord>,
    /// SQLite persistence store (memory FTS5, audit logs, budget tracking).
    pub(crate) db_store: Option<crate::ai::db::DbStore>,
    /// Budget tracker for token/turn limits.
    pub(crate) budget_tracker: Option<Arc<std::sync::Mutex<crate::ai::budget::BudgetTracker>>>,

    // ── Channel subsystem (Claw mode) ─────────────────────────────────
    /// Channel manager orchestrating all registered channels.
    /// `None` when Claw mode is disabled.
    pub(crate) channel_manager: Option<crate::channels::ChannelManager>,
    /// Shared system keychain secret store (single instance to avoid
    /// repeated OS permission dialogs).
    pub(crate) secret_store: Arc<crate::services::secret::SecretStore>,
    /// Pending reply targets for channel messages being processed by the LLM.
    /// Key is the LLM session_id (900000+), value is the ReplyTarget.
    pub(crate) pending_channel_replies: HashMap<u32, crate::channels::ReplyTarget>,
    /// Counter for channel message session IDs.
    pub(crate) _messages_received_channel_counter: u32,
    /// Channel conversation messages for display in channel tabs.
    /// Key is channel_id, value is ordered list of messages.
    pub(crate) channel_messages: HashMap<String, Vec<crate::channels::ChannelDisplayMessage>>,

    // ── Sandbox ──────────────────────────────────────────────────────────
    /// The sandbox policy that was applied at PTY spawn time.
    /// Compared against the current profile's policy to detect changes
    /// requiring a relaunch.
    pub(crate) applied_sandbox_policy: crate::profile::sandbox::policy::SandboxPolicy,

    // ── Agent execution infrastructure ───────────────────────────────────
    /// Shared pending results map for async skill result delivery.
    /// Skill handlers (browser/terminal) insert a oneshot::Sender under a
    /// UUID key; the main thread sends the result when it becomes available.
    pub(crate) pending_results: PendingResultMap,
    /// Pending terminal executions waiting for their marker to appear.
    pub(crate) pending_terminal_execs: Vec<PendingTerminalExec>,

    // ── Shared state for extended skills ──────────────────────────────────
    /// Shared tab info for skill queries (updated on tab create/close/switch).
    pub(crate) shared_tab_info: Arc<std::sync::Mutex<Vec<crate::ui::types::TabInfo>>>,
    /// Shared terminal info for skill queries (updated on terminal spawn/exit).
    pub(crate) shared_terminal_info: Arc<std::sync::Mutex<Vec<crate::ui::types::TerminalInfo>>>,
    /// Shared onboarding state for skill queries.
    pub(crate) shared_onboarding_state: crate::ai::skills::onboarding::OnboardingState,

    // ── Render loop state (event-driven rendering) ───────────────────────
    /// Frame scheduler — replaces needs_redraw / last_render_time / MIN_FRAME_INTERVAL.
    pub(crate) scheduler: crate::renderer::scheduler::FrameScheduler,
    /// Next cursor blink toggle deadline (None when blink disabled).
    pub(crate) next_cursor_blink: Option<std::time::Instant>,
    /// Whether the cursor is currently visible (blink phase).
    pub(crate) cursor_visible: bool,
}

impl AppState {
    /// Split `&mut AppState` into `(&mut Ui, AppCtx)` so that UI dispatch
    /// methods can hold `&mut self` on `Ui` while accessing non-UI state
    /// through `AppCtx`.
    pub(crate) fn split_ui(&mut self) -> (&mut crate::ui::Ui, crate::ui::model::AppCtx<'_>) {
        let cell_size = self.ui.frame.terminal.cell_size;
        (
            &mut self.ui,
            crate::ui::model::AppCtx {
                config: &mut self.config,
                ui_state: &mut self.ui_state,
                cell_size,
                sessions: &mut self.sessions,
                next_id: &mut self.next_id,
                scheduler: &mut self.scheduler,
                runtime: &self.runtime,
                proxy: &self.proxy,
                session_chats: &mut self.session_chats,
                llm_client: &mut self.llm_client,
                token_counter: &mut self.token_counter,
                skill_registry: &mut self.skill_registry,
                profile_manager: &self.profile_manager,
                agent_registry: &mut self.agent_registry,
                skills_manager: &self.skills_manager,
                loaded_external_skills: &mut self.loaded_external_skills,
                db_store: &self.db_store,
                secret_store: &self.secret_store,
                applied_sandbox_policy: &self.applied_sandbox_policy,
                pending_results: &mut self.pending_results,
                task_store: &mut self.task_store,
                scheduler_history_cache: &self.scheduler_history_cache,
                channel_manager: &mut self.channel_manager,
                channel_messages: &self.channel_messages,
                frame_count: &mut self.frame_count,
                prev_grid_sizes: &mut self.prev_grid_sizes,
                cursor_visible: self.cursor_visible,
            },
        )
    }

    /// Top-level UI-action dispatch — entry point for call sites in `app/` that
    /// hold `&mut AppState`.  Splits into `(Ui, AppCtx)` and delegates.
    pub(crate) fn dispatch_ui_action(&mut self, action: UiAction, event_loop: &ActiveEventLoop) {
        let (ui, mut ctx) = self.split_ui();
        ui.handle_ui_action(&mut ctx, action, event_loop);
    }

    /// Top-level colon-command dispatch.  Commands that need full `AppState`
    /// (`:ollama`, `:credentials`) are dispatched *before* the split; everything
    /// else goes through `Ui::handle_colon_command`.
    pub(crate) fn dispatch_colon_command(&mut self, cmd: &str, event_loop: &ActiveEventLoop) {
        let cmd = cmd.trim();
        let cmd = cmd.strip_prefix(':').unwrap_or(cmd);

        // ── Commands that require full AppState ──────────────────────
        if let Some(args) = cmd.strip_prefix("ollama") {
            let args = args.strip_prefix(' ').unwrap_or(args);
            crate::app::commands::ollama::handle_ollama_command(self, args);
            return;
        }
        if let Some(args) = cmd
            .strip_prefix("credentials")
            .or_else(|| cmd.strip_prefix("creds"))
        {
            let args = args.strip_prefix(' ').unwrap_or(args);
            crate::app::commands::credentials::handle_credentials_command(self, args);
            return;
        }

        // ── Everything else — split and route through Ui method ──────
        let (ui, mut ctx) = self.split_ui();
        ui.handle_colon_command(&mut ctx, cmd, event_loop);
    }
}
