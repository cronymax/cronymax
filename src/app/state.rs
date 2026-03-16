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

/// Actions that can be triggered by keybindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    NewChat,
    NewTerminal,
    CloseTab,
    NextTab,
    PrevTab,
    SplitVertical,
    SplitHorizontal,
    Copy,
    Paste,
    FontSizeUp,
    FontSizeDown,
    #[allow(dead_code)]
    ScrollUp,
    #[allow(dead_code)]
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
    CommandMode,
    ToggleFilter,
    ToggleSettings,
}

/// A single webview tab entry.
pub(crate) struct WebviewTab {
    pub(crate) id: WebviewId,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) manager: BrowserView,
    pub(crate) address_bar: AddressBarState,
    /// Display mode: Overlay (floating) or Docked (split).
    pub(crate) mode: BrowserViewMode,
    /// Terminal session this overlay is paired with (if any).
    pub(crate) paired_session: Option<SessionId>,
    /// Docked webview ID this overlay is paired with (if opened from a webview tab).
    pub(crate) paired_webview: Option<WebviewId>,
}

/// Application state — holds the window, GPU context, and all subsystems.
pub(crate) struct AppState {
    pub(crate) window: Arc<Window>,
    pub(crate) gpu: GpuContext,
    pub(crate) config: AppConfig,
    pub(crate) renderer: TerminalRenderer,
    /// All terminal sessions by ID.
    pub(crate) sessions: HashMap<SessionId, TerminalSession>,
    /// Tiling layout tree (egui_tiles).
    pub(crate) tile_tree: egui_tiles::Tree<tiles::Pane>,
    /// Tile rects collected each frame for wgpu viewport mapping & webview positioning.
    pub(crate) tile_rects: Vec<tiles::TileRect>,
    /// Previous grid dimensions per session (cols, rows) for PTY resize detection.
    pub(crate) prev_grid_sizes: HashMap<SessionId, (u16, u16)>,
    /// Next session ID counter.
    pub(crate) next_id: SessionId,
    pub(crate) viewport: ui::Viewport,
    pub(crate) modifiers: ModifiersState,
    /// Multiple webview tabs.
    pub(crate) webview_tabs: Vec<WebviewTab>,
    /// Active webview tab index.
    pub(crate) active_webview: usize,
    /// Next webview ID counter.
    pub(crate) next_webview_id: WebviewId,
    /// Centralized multi-child webview window manager (z-ordering,
    /// independent overlays, lifecycle management).
    pub(crate) webview_manager: WebviewManager,
    /// Per-frame transient state for the Float Panel tooltip rendering.
    pub(crate) float_panel_state: FloatPanelState,
    /// Float renderer (tier 3) — tooltip window above all overlays.
    /// Created lazily on first tooltip request.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) float_renderer: Option<crate::renderer::overlay::Float>,
    /// Overlay renderer (tier 2) for the Settings page.
    /// Created lazily when settings are opened.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) settings_overlay: Option<crate::renderer::overlay::Modal>,
    /// Split layout when webview is open.
    pub(crate) split: Option<VerticalSplit>,
    /// Accumulated input buffer for command mode.
    pub(crate) colon_buf: Option<String>,
    /// Reusable text buffer to reduce allocations in the render loop.
    pub(crate) text_scratch: String,
    /// Frame counter for diagnostics.
    pub(crate) frame_count: u64,
    /// Current mouse position in physical pixels.
    pub(crate) mouse_x: f32,
    pub(crate) mouse_y: f32,
    /// Currently hovered link (when Cmd/Ctrl held + mouse over link).
    pub(crate) hovered_link: Option<crate::terminal::links::DetectedLink>,
    /// Whether an IME composition (preedit) is currently active.
    pub(crate) ime_composing: bool,
    /// Whether the IME input method is enabled (between Ime::Enabled and Ime::Disabled).
    /// Used to suppress the first KeyboardInput character event that arrives
    /// before Ime::Preedit when starting a new CJK composition.
    pub(crate) ime_enabled: bool,

    // ── egui integration ──────────────────────────────────────────────────
    /// egui context + winit adaptor + custom wgpu renderer.
    pub(crate) egui: EguiIntegration,
    /// Centralized UI widget state (tabs, overlay, filter, address bar).
    pub(crate) ui_state: UiState,
    /// theme (colors + spacing).
    pub(crate) styles: Styles,
    /// Per-session input line state (Editor mode).
    pub(crate) prompt_editors: HashMap<SessionId, PromptState>,
    /// Persistent pane widget instances (stateful widget tree).
    pub(crate) pane_widgets: tiles::PaneWidgetStore,

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
    /// Settings overlay state.
    pub(crate) settings_state: crate::ui::settings::SettingsState,
    /// UI state for the General settings section.
    pub(crate) general_ui_state: crate::ui::settings::general::GeneralSettingsState,
    /// Agent registry for installable agent packages.
    pub(crate) agent_registry: crate::ai::agent::AgentRegistry,
    /// UI state for the Agents & Skills settings section.
    pub(crate) agents_ui_state: crate::ui::settings::agents::AgentsSettingsState,
    /// Skills manager for OpenClaw external skills lifecycle.
    pub(crate) skills_manager: Option<crate::ai::skills::manager::SkillsManager>,
    /// Loaded external skills (cached for system prompt injection).
    pub(crate) loaded_external_skills: Vec<crate::ai::skills::loader::ExternalSkill>,
    /// Skills panel UI state.
    pub(crate) skills_panel_state: crate::ui::skills_panel::SkillsPanelState,
    /// UI state for the Profiles settings section.
    pub(crate) profiles_ui_state: crate::ui::settings::profiles::ProfilesSettingsState,
    /// UI state for the LLM Providers settings section.
    pub(crate) providers_ui_state: crate::ui::settings::providers::ProvidersSettingsState,
    /// Scheduled task store.
    pub(crate) task_store: crate::ai::scheduler::ScheduledTaskStore,
    /// UI state for the Scheduler settings section.
    pub(crate) scheduler_ui_state: crate::ui::settings::scheduler::SchedulerSettingsState,
    /// Cached execution history for the scheduler UI.
    pub(crate) scheduler_history_cache: Vec<crate::ai::scheduler::ExecutionRecord>,
    /// SQLite persistence store (memory FTS5, audit logs, budget tracking).
    pub(crate) db_store: Option<crate::ai::db::DbStore>,
    /// Budget tracker for token/turn limits.
    pub(crate) budget_tracker: Option<Arc<std::sync::Mutex<crate::ai::budget::BudgetTracker>>>,

    // ── Channel subsystem (Claw mode) ─────────────────────────────────
    /// Channel manager orchestrating all registered channels.
    /// `None` when Claw mode is disabled.
    pub(crate) channel_manager: Option<crate::channel::ChannelManager>,
    /// UI state for the Channels settings section.
    pub(crate) channels_ui_state: crate::ui::settings::channels::ChannelsSettingsState,
    /// Onboarding wizard state (visible when first enabling Claw with no channels).
    pub(crate) onboarding_wizard_state:
        Option<crate::ui::settings::onboarding::OnboardingWizardState>,
    /// Shared system keychain secret store (single instance to avoid
    /// repeated OS permission dialogs).
    pub(crate) secret_store: Arc<crate::secret::SecretStore>,
    /// Pending reply targets for channel messages being processed by the LLM.
    /// Key is the LLM session_id (900000+), value is the ReplyTarget.
    pub(crate) pending_channel_replies: HashMap<u32, crate::channel::ReplyTarget>,
    /// Counter for channel message session IDs.
    pub(crate) _messages_received_channel_counter: u32,
    /// Channel conversation messages for display in channel tabs.
    /// Key is channel_id, value is ordered list of messages.
    pub(crate) channel_messages: HashMap<String, Vec<crate::channel::ChannelDisplayMessage>>,

    // ── Sandbox ──────────────────────────────────────────────────────────
    /// The sandbox policy that was applied at PTY spawn time.
    /// Compared against the current profile's policy to detect changes
    /// requiring a relaunch.
    pub(crate) applied_sandbox_policy: crate::sandbox::policy::SandboxPolicy,

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

/// Unique webview tab identifier.
pub(super) type WebviewId = u32;
