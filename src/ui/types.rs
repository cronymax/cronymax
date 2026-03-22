//! UI widget type definitions — unified data model for the widget hierarchy.
//!
//! # Widget Hierarchy
//!
//! ```text
//! 1. Titlebar                       ← window br: macOS controls, pinned tabs, actions
//! 2. Tiles                          ← main content area
//!    2.1 Tabs Bar                   ← tab strip with right-side actions
//!    2.2 Pane Tree                  ← leaf content panes (egui_tiles)
//!        2.2.1 Chat pane            ← chat: egui prompt + PTY output grid
//!        2.2.2 Terminal pane        ← raw PTY terminal
//!        2.2.3 BrowserView pane     ← docked webview with address bar
//!        2.2.4 Channel pane         ← messaging channel conversation
//!    2.3 Block                      ← content units inside a pane
//!        2.3.1 Terminal block       ← PTY command + output (BlockMode::Terminal)
//!        2.3.2 Stream block         ← SSE/LLM exchange (BlockMode::Stream)
//!    2.4 Prompt                     ← input area at pane bottom
//!        - Suggestion panel         ← commands / file picker
//!        - Prompt editor            ← context bar, text edit, hint bar
//! 3. Overlay (child window)         ← floating panels above main content
//!    3.1 BrowserView overlay        ← overlay webview (ModalPanel + ChildWindowGpu)
//!    3.2 Settings overlay           ← settings page
//! 4. Float (child window)           ← highest z-order, above overlays
//!    4.1 Tooltips                   ← hover tooltips (FloatPanel)
//!    4.2 Dialogs                    ← modal dialogs (todo)
//! 5. BrowserView                    ← shared browser view
//!    - Address bar                  ← URL + navigation buttons
//!    - Webview                      ← native WKWebView / WebView2
//! ```
//!
//! See [`PaneKind`] for pane variants, [`OverlayKind`] for overlay variants,
//! and [`FloatKind`] for float-layer content types.

// ─── Tooltip Request ─────────────────────────────────────────────────────────

use std::{collections::HashMap, sync::Arc};

use crate::{
    config::AppConfig, renderer::terminal::SessionId, services::secret::SecretStore, ui::{prompt::PromptState, tiles}
};

/// Tooltip request emitted by overlay browser rendering.
///
/// Produced by `ChildWindowGpu::render_browser()` when a browser button is hovered,
/// consumed by the Float Panel render pass in `app.rs` to render the tooltip
/// above all native webview content.
#[derive(Debug, Clone)]
pub struct TooltipRequest {
    /// Screen-space X position (logical, center of tooltip).
    pub screen_x: f32,
    /// Screen-space Y position (logical, top of tooltip).
    pub screen_y: f32,
    /// Tooltip text content.
    pub text: String,
}

// ─── Widget Hierarchy Enums ──────────────────────────────────────────────────

/// Leaf pane content kind — see widget hierarchy §2.2.
///
/// Maps 1:1 to [`crate::ui::tiles::Pane`] enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PaneKind {
    /// Chat pane (§2.2.1) — egui prompt + PTY output grid.
    Chat,
    /// Terminal pane (§2.2.2) — raw PTY, no prompt editor.
    Terminal,
    /// Docked browser view pane (§2.2.3).
    BrowserView,
    /// Channel conversation pane (§2.2.4).
    Channel,
}

/// Overlay variant — see widget hierarchy §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum OverlayKind {
    /// Browser view overlay (§3.1) — ModalPanel with address bar browser.
    BrowserView,
    /// Settings overlay (§3.2) — full-page settings panel.
    Settings,
}

/// Float-layer content kind — see widget hierarchy §4.
///
/// Float windows have the highest z-order (float > overlay > main window).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FloatKind {
    /// Hover tooltip (§4.1) — non-focusable, click-through.
    Tooltip,
    /// Modal dialog (§4.2) — focusable, blocks input (todo).
    Dialog,
}

// ─── BrowserView Display Mode ────────────────────────────────────────────────

/// How a browser view is displayed: floating overlay or docked as a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserViewMode {
    /// Centered floating overlay (initial state).
    #[default]
    Overlay,
    /// Docked as a split alongside the terminal.
    Docked,
}

// ─── UiState ─────────────────────────────────────────────────────────────────

/// Centralized mutable state for all egui UI panels.
#[derive(Debug)]
pub struct UiState {
    /// All tabs (terminal + browser) in display order.
    pub tabs: Vec<TabInfo>,
    /// Active tab index (into `tabs`).
    pub active_tab: usize,

    /// Which browser tab is focused (index into AppState::browser_tabs).
    pub active_browser: Option<usize>,
    /// ID of the active browser (derived in sync_ui_state for UI lookups).
    pub active_browser_id: Option<u32>,

    /// Session IDs of pinned tabs (displayed in the titlebar).
    pub pinned_tabs: Vec<u32>,

    /// The terminal session that last received a pointer click.
    /// Used to route keyboard input to the correct split pane.
    pub focused_terminal_session: Option<u32>,

    /// Command overlay state.
    pub command_suggestions: CommandSuggestionsState,

    /// Inline filter bar state.
    pub filter: FilterState,

    /// Address bar state (for browser).
    pub address_bar: AddressBarState,

    /// The actual egui-computed rect (in logical pixels) for the browser view
    /// overlay content area (below the address bar, inset from borders).
    /// Set each frame by `draw_browser_overlay` so `app.rs` can position the
    /// native Browser without duplicating geometry calculations.
    pub overlay_content_rect: Option<[f32; 4]>,

    /// The full overlay panel rect (in logical pixels) including the address
    /// bar and border areas.  Used to position the NSPanel / popover window
    /// so it covers the entire overlay, preventing docked browsers from
    /// showing through the egui-rendered border.
    pub overlay_panel_rect: Option<[f32; 4]>,

    /// Tooltip request from a docked browser address bar hover.
    /// Set by [`TilesPanel::show()`] each frame; consumed by the app layer
    /// to route through the FloatPanel system.
    pub docked_tooltip: Option<TooltipRequest>,

    /// Whether Claw mode (channels) is enabled — controls Feishu icon visibility.
    pub claw_enabled: bool,
    /// Current channel connection state (for titlebar status indicator).
    pub channel_connection_state: crate::channels::ConnectionState,

    // ── Profile picker (titlebar) ─────────────────────────────────────────
    /// Available profiles: `(id, display_name)` pairs.
    pub profile_list: Vec<(String, String)>,
    /// Active profile ID (empty string if none).
    pub active_profile_id: String,
    /// Whether a profile-switch relaunch dialog should be shown.
    pub show_profile_relaunch_dialog: bool,

    pub prompt_editors: HashMap<SessionId, PromptState>,
    pub pane_widgets: tiles::PaneWidgetStore,
    pub settings_state: crate::ui::settings::SettingsState,
    pub providers_ui_state: crate::ui::settings::providers::ProvidersSettingsState,
    pub general_ui_state: crate::ui::settings::general::GeneralSettingsState,
    pub channels_ui_state: crate::ui::settings::channels::ChannelsSettingsState,
    pub onboarding_wizard_state: Option<crate::ui::settings::onboarding::OnboardingWizardState>,
    pub agents_ui_state: crate::ui::settings::agents::AgentsSettingsState,
    pub profiles_ui_state: crate::ui::settings::profiles::ProfilesSettingsState,
    pub scheduler_ui_state: crate::ui::settings::scheduler::SchedulerSettingsState,
    pub skills_panel_state: crate::ui::skills_panel::SkillsPanelState,
}

/// Unified tab entry — one per tab in the titlebar / tab bar.
///
/// Covers all pane kinds (§2.2). Chat tabs (§2.2.1) are the default;
/// terminal tabs (§2.2.2) run a raw PTY without the prompt editor.
#[derive(Debug, Clone)]
pub enum TabInfo {
    /// Chat tab (§2.2.1) — egui prompt + PTY output grid.
    Chat { session_id: u32, title: String },
    /// Terminal tab (§2.2.2) — raw PTY, no prompt editor.
    Terminal { session_id: u32, title: String },
    /// Browser view tab (docked or overlay, determined by `mode`).
    BrowserView {
        webview_id: u32,
        title: String,
        url: String,
        mode: BrowserViewMode,
    },
    /// Channel conversation tab.
    #[allow(dead_code)]
    Channel {
        channel_id: String,
        channel_name: String,
    },
}

impl TabInfo {
    /// Numeric ID (session_id for chat/terminal, webview_id for browser views, 0 for channels).
    pub fn id(&self) -> u32 {
        match self {
            Self::Chat { session_id, .. } | Self::Terminal { session_id, .. } => *session_id,
            Self::BrowserView { webview_id, .. } => *webview_id,
            Self::Channel { .. } => 0,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Chat { title, .. }
            | Self::Terminal { title, .. }
            | Self::BrowserView { title, .. } => title,
            Self::Channel { channel_name, .. } => channel_name,
        }
    }

    /// True for chat tabs (§2.2.1).
    pub fn is_chat(&self) -> bool {
        matches!(self, Self::Chat { .. })
    }

    /// True for terminal tabs (§2.2.2).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal { .. })
    }

    /// True for any session-based tab (chat or terminal).
    pub fn is_session(&self) -> bool {
        matches!(self, Self::Chat { .. } | Self::Terminal { .. })
    }

    /// True for browser view tabs (both overlay and docked modes).
    pub fn is_browser_view(&self) -> bool {
        matches!(self, Self::BrowserView { .. })
    }

    /// True for channel tabs.
    #[allow(dead_code)]
    pub fn is_channel(&self) -> bool {
        matches!(self, Self::Channel { .. })
    }

    /// The semantic pane kind for this tab entry.
    #[allow(dead_code)]
    pub fn pane_kind(&self) -> PaneKind {
        match self {
            Self::Chat { .. } => PaneKind::Chat,
            Self::Terminal { .. } => PaneKind::Terminal,
            Self::BrowserView { .. } => PaneKind::BrowserView,
            Self::Channel { .. } => PaneKind::Channel,
        }
    }

    // ── Backwards-compatible aliases ──

    /// Alias for [`is_browser_view`](Self::is_browser_view) (legacy name).
    pub fn is_webview(&self) -> bool {
        self.is_browser_view()
    }
}

/// Lightweight browser view metadata used by AI skills (separate from UiState tabs).
#[derive(Debug, Clone)]
pub struct BrowserViewInfo {
    pub title: String,
    pub url: String,
    pub webview_id: u32,
    pub mode: BrowserViewMode,
}

/// Terminal session metadata for AI skill queries.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TerminalInfo {
    pub session_id: u32,
    pub title: String,
    pub pid: Option<u32>,
    pub cwd: Option<String>,
    pub running: bool,
}

#[derive(Debug, Default)]
pub struct CommandSuggestionsState {
    pub open: bool,
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub label: String,
    pub action: String,
    pub shortcut: Option<String>,
    /// If true, the command expects additional arguments (e.g. `:webview <url>`).
    /// When false, selecting the command auto-submits immediately.
    pub needs_arg: bool,
}

// ─── Filter ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FilterState {
    pub open: bool,
    pub text: String,
    pub match_count: usize,
    pub current_match: usize,
}

impl FilterState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.text.clear();
            self.match_count = 0;
            self.current_match = 0;
        }
    }
}

// ─── Address Bar ─────────────────────────────────────────────────────────────

/// State for the webview address bar (§5).
///
/// Used both by `UiState` (overlay address bar) and per-`WebviewTab`
/// (docked address bar). Previously duplicated in `browser.rs`.
#[derive(Debug, Default)]
pub struct AddressBarState {
    pub url: String,
    pub editing: bool,
    pub edit_text: String,
    pub cursor_pos: usize,
}

impl AddressBarState {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            editing: false,
            edit_text: String::new(),
            cursor_pos: 0,
        }
    }

    pub fn start_editing(&mut self) {
        self.editing = true;
        self.edit_text = self.url.clone();
        self.cursor_pos = self.edit_text.len();
    }

    pub fn stop_editing(&mut self) {
        self.editing = false;
    }

    pub fn confirm(&mut self) -> String {
        self.editing = false;
        self.url = self.edit_text.clone();
        self.url.clone()
    }

    pub fn update_url(&mut self, url: &str) {
        self.url = url.to_string();
        if !self.editing {
            self.edit_text = url.to_string();
        }
    }
}

// ─── UiState Constructors ────────────────────────────────────────────────────

impl UiState {
    pub fn new(app_config: &AppConfig, shared_secret_store: Arc<SecretStore>) -> Self {
        // Per-session prompt editors.
        let mut prompt_editors = HashMap::new();
        let mut prompt_editor = crate::ui::prompt::PromptState::new();
        prompt_editor.visible = true;
        prompt_editors.insert(1_u32, prompt_editor);

        Self {
            tabs: Vec::new(),
            active_tab: 0,
            active_browser: None,
            active_browser_id: None,
            pinned_tabs: Vec::new(),
            focused_terminal_session: None,
            command_suggestions: CommandSuggestionsState {
                open: false,
                commands: default_commands(),
            },
            filter: FilterState::default(),
            address_bar: AddressBarState::default(),
            overlay_content_rect: None,
            overlay_panel_rect: None,
            docked_tooltip: None,
            claw_enabled: false,
            channel_connection_state: crate::channels::ConnectionState::Disconnected,
            profile_list: Vec::new(),
            active_profile_id: String::new(),
            show_profile_relaunch_dialog: false,

            prompt_editors,
            pane_widgets: tiles::PaneWidgetStore::default(),
            settings_state: crate::ui::settings::SettingsState::default(),
            general_ui_state: {
                let claw_enabled = app_config.claw.as_ref().is_some_and(|c| c.enabled);
                crate::ui::settings::general::GeneralSettingsState::new(claw_enabled)
            },
            providers_ui_state:
                crate::ui::settings::providers::ProvidersSettingsState::with_secret_store(
                    shared_secret_store.clone(),
                ),
            scheduler_ui_state: crate::ui::settings::scheduler::SchedulerSettingsState::default(),
            channels_ui_state:
                crate::ui::settings::channels::ChannelsSettingsState::from_claw_config_with_store(
                    app_config.claw.as_ref(),
                    shared_secret_store.clone(),
                ),
            agents_ui_state: crate::ui::settings::agents::AgentsSettingsState::default(),
            skills_panel_state: crate::ui::skills_panel::SkillsPanelState::new(),
            profiles_ui_state: crate::ui::settings::profiles::ProfilesSettingsState::default(),
            onboarding_wizard_state: None,
        }
    }
}

pub(crate) fn default_commands() -> Vec<CommandEntry> {
    vec![
        CommandEntry {
            label: "New Chat Tab".into(),
            action: "newtab".into(),
            shortcut: Some("Ctrl+T".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "New Terminal Tab".into(),
            action: "new_terminal_tab".into(),
            shortcut: None,
            needs_arg: false,
        },
        CommandEntry {
            label: "Close Tab".into(),
            action: "closetab".into(),
            shortcut: Some("Ctrl+W".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Find in Terminal".into(),
            action: "filter".into(),
            shortcut: Some("Ctrl+F".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Split Horizontal".into(),
            action: "split_horizontal".into(),
            shortcut: Some("Ctrl+Shift+H".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Split Vertical".into(),
            action: "split_vertical".into(),
            shortcut: Some("Ctrl+Shift+V".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Open Webview".into(),
            action: "webview".into(),
            shortcut: None,
            needs_arg: true,
        },
        CommandEntry {
            label: "Close".into(),
            action: "close".into(),
            shortcut: None,
            needs_arg: false,
        },
        CommandEntry {
            label: "Copilot Login".into(),
            action: "copilot-login".into(),
            shortcut: None,
            needs_arg: false,
        },
        CommandEntry {
            label: "Ollama Models".into(),
            action: "ollama".into(),
            shortcut: None,
            needs_arg: true,
        },
        CommandEntry {
            label: "Ollama Install".into(),
            action: "ollama install".into(),
            shortcut: None,
            needs_arg: false,
        },
        CommandEntry {
            label: "Ollama Serve".into(),
            action: "ollama serve".into(),
            shortcut: None,
            needs_arg: false,
        },
        CommandEntry {
            label: "Ollama List".into(),
            action: "ollama list".into(),
            shortcut: None,
            needs_arg: false,
        },
        CommandEntry {
            label: "Ollama Pull".into(),
            action: "ollama pull".into(),
            shortcut: None,
            needs_arg: true,
        },
        CommandEntry {
            label: "Pull llama3.1".into(),
            action: "ollama pull llama3.1".into(),
            shortcut: Some("8B".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Pull gemma3".into(),
            action: "ollama pull gemma3".into(),
            shortcut: Some("4B".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Pull qwen3".into(),
            action: "ollama pull qwen3".into(),
            shortcut: Some("8B".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Pull mistral".into(),
            action: "ollama pull mistral".into(),
            shortcut: Some("7B".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Pull deepseek-r1".into(),
            action: "ollama pull deepseek-r1".into(),
            shortcut: Some("7B".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Pull codellama".into(),
            action: "ollama pull codellama".into(),
            shortcut: Some("13B".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Pull phi4".into(),
            action: "ollama pull phi4".into(),
            shortcut: Some("14B".into()),
            needs_arg: false,
        },
        CommandEntry {
            label: "Ollama Status".into(),
            action: "ollama status".into(),
            shortcut: None,
            needs_arg: false,
        },
        CommandEntry {
            label: "Credentials".into(),
            action: "credentials".into(),
            shortcut: None,
            needs_arg: true,
        },
    ]
}
