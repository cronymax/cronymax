/// Actions the UI wants the app to perform.
#[derive(Debug, Clone)]
pub enum UiAction {
    /// Switch to terminal tab at index.
    #[allow(dead_code)]
    SwitchTab(usize),
    /// Close terminal tab by session ID.
    CloseTab(u32),
    /// Create a new chat tab.
    NewChat,
    /// Create a new terminal tab.
    NewTerminal,
    /// Switch to webview tab at index (for overlay/docked webviews not in tile tree).
    #[allow(dead_code)]
    SwitchWebview(usize),
    /// Activate a webview pane in the tile tree by webview ID.
    #[allow(dead_code)]
    ActivateWebviewPane(u32),
    /// Close webview tab by webview ID.
    CloseWebview(u32),
    /// Execute a command from the overlay.
    #[allow(dead_code)]
    ExecuteCommand(String),
    /// Navigate webview to URL. The u32 is the target webview ID (0 = active).
    NavigateWebview(String, u32),
    /// Open a URL in a new webview tab (used by AI skills).
    #[allow(dead_code)]
    OpenWebviewTab(String),
    /// Close webview tab by webview ID.
    CloseWebviewTab(u32),
    /// Install an agent from a local directory (triggers file dialog).
    InstallAgent,
    /// Uninstall an agent by name.
    UninstallAgent(String),
    /// Toggle an agent's enabled/disabled state.
    ToggleAgent(String),
    /// Create a new profile from the settings form.
    CreateProfile,
    /// Save the edited profile by ID.
    SaveProfile(String),
    /// Duplicate a profile by source ID.
    DuplicateProfile(String),
    /// Delete a profile by ID.
    DeleteProfile(String),
    /// Set a profile as the active profile.
    SetActiveProfile(String),
    /// Create a new scheduled task.
    CreateScheduledTask,
    /// Save an edited scheduled task by ID.
    SaveScheduledTask(String),
    /// Delete a scheduled task by ID.
    DeleteScheduledTask(String),
    /// Toggle a scheduled task's enabled state by ID.
    ToggleScheduledTask(String),
    /// Webview navigation: back, forward, refresh.
    /// The u32 is the target webview ID (0 = active).
    WebviewBack(u32),
    WebviewForward(u32),
    WebviewRefresh(u32),
    /// Dock the active overlay webview as a split (default: right).
    #[allow(dead_code)]
    DockWebview,
    /// Dock the active overlay webview as a split — specific direction.
    #[allow(dead_code)]
    DockWebviewLeft,
    DockWebviewRight,
    DockWebviewDown,
    /// Move a webview into/out of a tab (toggle overlay ↔ docked tile).
    /// The u32 is the webview ID (0 = sentinel replaced by caller).
    WebviewToTab(u32),
    /// Open the current webview URL in the system browser.
    OpenInSystemBrowser,
    /// Submit filter text search.
    FilterSearch(String),
    /// Dismiss filter bar.
    FilterClose,
    /// Filter: go to next/previous match.
    FilterNext,
    FilterPrev,
    /// Dock a dragged tab as a split relative to a target pane.
    DockTab {
        source: crate::terminal::SessionId,
        target: crate::terminal::SessionId,
        direction: crate::ui::tiles::DockDirection,
    },
    /// Split active pane horizontally, new pane to the left.
    #[allow(dead_code)]
    SplitLeft,
    /// Split active pane horizontally, new pane to the right.
    SplitRight,
    /// Split active pane vertically, new pane below.
    SplitDown,
    /// Pin a tab to the titlebar by session ID.
    PinTab(u32),
    /// Unpin a tab from the titlebar by session ID.
    UnpinTab(u32),
    /// Window management: initiate native OS window drag.
    StartWindowDrag,
    /// Window management: close the application.
    CloseWindow,
    /// Window management: minimize the window.
    Minimize,
    /// Window management: toggle maximize/restore.
    ToggleMaximize,
    /// Pop out the active overlay webview into an independent child window.
    /// The independent window gets its own egui browser view (address bar, buttons).
    PopOutOverlay,
    /// Bring an overlay/independent webview to the front of the z-stack.
    #[allow(dead_code)]
    BringOverlayToFront(u32),
    /// Open the Settings overlay page.
    OpenSettings,
    /// Close the Settings overlay page.
    CloseSettings,
    /// Popup an overlay webview with the default (home) page.
    OpenOverlay,
    /// Relaunch the application (e.g. after sandbox rules change).
    RelaunchApp,
    /// Launch a new window with a specific profile ID.
    NewWindowWithProfile(String),
    /// Save the configured LLM providers to config.toml.
    SaveProviders,
    /// Switch the active LLM model for a session.
    SwitchModel {
        session_id: u32,
        provider: String,
        model: String,
        display_label: String,
    },
    /// Enable Claw mode (channels subsystem).
    EnableClawMode,
    /// Disable Claw mode (channels subsystem).
    DisableClawMode,
    /// Save the channel (Lark) configuration to config.toml.
    SaveChannelConfig,
    /// Save a specific channel instance configuration by instance_id.
    SaveChannelConfigById {
        instance_id: String,
    },
    /// Onboarding wizard: step changed (persists to DB).
    OnboardingWizardStepChanged {
        step: String,
    },
    /// Onboarding wizard: completed with channel configuration.
    OnboardingWizardComplete {
        app_id: String,
        app_secret: Option<String>,
        app_secret_env: String,
        api_base: String,
        allowed_users: Vec<String>,
        profile_id: String,
        secret_storage: crate::secret::SecretStorage,
    },
    /// Run a best-effort Lark Developer Console automation in the browser overlay.
    OnboardingAutomateLarkSetup {
        app_id: String,
    },
    /// Toggle the Lark channel on/off from the Channels settings section.
    ToggleLarkChannel(bool),
    /// Toggle a specific Lark channel instance on/off by instance_id.
    ToggleLarkChannelById {
        instance_id: String,
        enabled: bool,
    },
    /// Explicitly start the onboarding wizard (e.g. from Channels section button).
    StartOnboarding,
    /// Test the Lark channel connection (validates credentials + WebSocket).
    TestChannelConnection,
    /// Test a specific Lark channel instance connection by instance_id.
    TestChannelConnectionById {
        instance_id: String,
    },
    /// Add a new Lark channel instance.
    AddLarkChannel,
    /// Remove a Lark channel instance by instance_id.
    RemoveLarkChannel {
        instance_id: String,
    },
    /// Toggle starred state for a pane block message.
    ToggleStarred {
        session_id: u32,
        message_id: u32,
    },
    /// Close a channel tab.
    CloseChannel(String),
    /// Open a channel as a tab in the tile tree.
    OpenChannelTab {
        channel_id: String,
        channel_name: String,
    },
    // ── Skills ──────────────────────────────────────────────────────────
    /// Install a skill from ClawHub by slug.
    InstallSkill(String),
    /// Uninstall a local skill by name.
    UninstallSkill(String),
    /// Toggle a skill's enabled/disabled state by name.
    ToggleSkill {
        name: String,
        enabled: bool,
    },
    /// Search ClawHub for skills (query text).
    SearchSkills(String),
    /// Reload all skills from disk.
    ReloadSkills,
    /// Open / close the skills management panel.
    ToggleSkillsPanel,
}
