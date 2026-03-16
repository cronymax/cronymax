// SSE streaming — AppEvent definitions for LLM token streaming.
//
// These events flow from tokio LLM streaming tasks back to the winit event loop
// via `EventLoopProxy<AppEvent>`.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Shared map for pending async skill results (script injection, terminal exec, etc.).
/// Skill handlers insert a `oneshot::Sender` under a UUID key; the main thread
/// sends the result through the channel when it becomes available.
pub type PendingResultMap =
    Arc<std::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>>;

/// Token usage reported by the LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Application events sent from background tasks to the winit event loop.
///
/// Delivered via `EventLoopProxy<AppEvent>` and handled in `App::user_event()`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A streamed token arrived from the LLM.
    LlmToken { session_id: u32, token: String },
    /// LLM streaming completed successfully.
    LlmDone {
        session_id: u32,
        /// Full response content (reconstructed from tokens).
        full_response: String,
        /// Provider-reported token usage.
        usage: Option<TokenUsage>,
        /// Tool calls requested by the LLM (OpenAI function calling).
        tool_calls: Vec<ToolCallInfo>,
    },
    /// LLM streaming failed.
    LlmError { session_id: u32, error: String },
    /// Context window is at ≥80% capacity; summarization may be warranted.
    CompactionNeeded {
        session_id: u32,
        used_tokens: u32,
        limit_tokens: u32,
    },
    /// LLM invoked a skill tool; execution result returned.
    ToolResult {
        session_id: u32,
        tool_call_id: String,
        result: String,
    },
    /// A skill requests a UI mutation on the main thread.
    SkillUiAction {
        session_id: u32,
        tool_call_id: String,
        action: crate::ui::UiAction,
        /// JSON result string fed back to the LLM after the action executes.
        result: String,
    },
    /// A scheduled task has started executing.
    ScheduledTaskStarted { task_id: String, task_name: String },
    /// A scheduled task has completed.
    ScheduledTaskCompleted {
        task_id: String,
        task_name: String,
        status: String,
        duration_ms: u64,
        /// Captured stdout/stderr for command tasks; empty for prompt tasks.
        output: String,
    },
    /// Available models fetched from provider API(s).
    ModelsLoaded {
        models: Vec<crate::ai::client::ModelListItem>,
    },
    /// Inject a JavaScript snippet into a webview and track the result.
    InjectScript {
        webview_id: u32,
        script: String,
        request_id: String,
    },
    /// Execute a command in a terminal session with marker-based output capture.
    TerminalExec {
        terminal_id: usize,
        command: String,
        marker: String,
        timeout_ms: u64,
    },
    /// Read the screen content of a terminal session.
    ReadTerminalScreen {
        terminal_id: usize,
        start_line: Option<i32>,
        end_line: Option<i32>,
        max_lines: usize,
        request_id: String,
    },

    // ── Copilot OAuth device-flow events ─────────────────────────────────
    /// Device code obtained — UI should open the verification URL.
    CopilotDeviceCode {
        user_code: String,
        verification_uri: String,
    },
    /// Copilot login completed — client should be updated.
    CopilotLoginComplete {
        oauth_token: String,
        session_token: String,
        api_base: String,
    },
    /// Copilot login failed.
    CopilotLoginFailed { error: String },

    // ── Channel subsystem events (Claw mode) ─────────────────────────────
    /// A message was received from a channel (already authorized).
    ChannelMessageReceived {
        message: crate::channels::ChannelMessage,
    },

    /// Request to send a reply through a channel.
    ChannelSendReply {
        target: crate::channels::ReplyTarget,
        content: String,
    },

    /// Channel connection status changed.
    ChannelStatusChanged {
        channel_id: String,
        status: crate::channels::ChannelStatus,
    },

    /// Result of a channel connection test.
    ChannelTestResult { success: bool, message: String },

    /// Result of a comprehensive bot configuration check.
    ChannelBotCheckResult {
        results: Vec<crate::channels::BotCheckResult>,
    },

    /// Instance-scoped channel test result.
    ChannelTestResultById {
        instance_id: String,
        success: bool,
        message: String,
    },

    /// Instance-scoped bot configuration check result.
    ChannelBotCheckResultById {
        instance_id: String,
        results: Vec<crate::channels::BotCheckResult>,
    },

    // ── Skills events ────────────────────────────────────────────────────
    /// Reload external skills from the filesystem (after install/uninstall).
    ReloadSkills,

    /// ClawHub search results arrived.
    SkillSearchResults(Vec<crate::ai::clawhub::ClawHubSkillResult>),

    /// ClawHub search failed.
    SkillSearchError(String),

    // ── Extended skill events (skills-reimpl) ────────────────────────────
    /// Find files in the workspace by name pattern.
    FindFiles {
        query: String,
        cwd: String,
        max_results: usize,
        request_id: String,
    },

    /// Star a chat block message.
    StarChatBlock { session_id: u32, message_id: u32 },

    /// Unstar a chat block message.
    UnstarChatBlock { session_id: u32, message_id: u32 },

    /// Read terminal output for reference.
    ReferenceTerminalOutput {
        terminal_id: u32,
        start_line: Option<i32>,
        end_line: Option<i32>,
        request_id: String,
    },

    /// Read a chat block's content.
    ReferenceBlockContent {
        session_id: u32,
        message_id: u32,
        request_id: String,
    },

    /// Add context to the active session.
    AddContext {
        session_id: u32,
        content: String,
        label: Option<String>,
    },

    /// Remove a context message by ID.
    RemoveContext { session_id: u32, message_id: u32 },

    /// Compact the context window (summarize old messages).
    CompactContext { session_id: u32, request_id: String },

    /// Rename a tab.
    RenameTab { session_id: u32, title: String },

    /// Start onboarding wizard.
    OnboardingStart {
        channel_type: String,
        instance_id: Option<String>,
        request_id: String,
    },

    /// Advance onboarding wizard step.
    OnboardingAdvanceStep { action: String, request_id: String },

    /// Test channel connection during onboarding.
    OnboardingTestConnection { request_id: String },

    /// Store channel credentials during onboarding.
    OnboardingStoreCredentials {
        app_id: String,
        app_secret: String,
        request_id: String,
    },

    /// Finalize channel onboarding — save config, start channel, set claw_enabled.
    /// Triggered by the LLM skill after successful credential storage + connection test.
    OnboardingFinalize { request_id: String },

    /// Browser automation finished for the onboarding wizard.
    OnboardingBrowserAutomationFinished { success: bool, message: String },

    // ── Scheduler ────────────────────────────────────────────────────────
    /// Create a new scheduled task.
    SchedulerCreate {
        name: String,
        cron: String,
        action_type: String,
        action_value: String,
        agent_name: String,
        enabled: bool,
        run_once: bool,
        request_id: String,
    },

    /// A scheduled task is due — execute its action on the main thread.
    ScheduledTaskFire {
        task_id: String,
        task_name: String,
        action_type: String,
        action_value: String,
    },

    /// List all scheduled tasks.
    SchedulerList { request_id: String },

    /// Get a single scheduled task by ID.
    SchedulerGet { task_id: String, request_id: String },

    /// Delete a scheduled task.
    SchedulerDelete { task_id: String, request_id: String },

    /// Toggle a task's enabled state.
    SchedulerToggle { task_id: String, request_id: String },

    /// Update fields of an existing task.
    SchedulerUpdate {
        task_id: String,
        name: Option<String>,
        cron: Option<String>,
        action_type: Option<String>,
        action_value: Option<String>,
        agent_name: Option<String>,
        enabled: Option<bool>,
        request_id: String,
    },

    // ── Ollama management events ─────────────────────────────────────────
    /// Ollama command result or info message to display in chat.
    OllamaInfoMessage {
        session_id: Option<u32>,
        text: String,
    },

    /// Throttled pull progress update.
    OllamaPullProgress {
        session_id: Option<u32>,
        text: String,
    },

    /// Model pull completed (triggers model list refresh).
    OllamaPullComplete { model: String },

    // ── Render loop events (event-driven rendering) ──────────────────────
    /// PTY reader thread detected data; main thread should drain try_recv().
    PtyDataReady { session_id: u32 },

    /// Immediate repaint requested (egui callback or internal).
    RequestRepaint,

    /// Deferred repaint requested (e.g. cursor blink timer).
    RequestRepaintAfter { delay: std::time::Duration },
}

/// Information about a tool call from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub id: String,
    pub function_name: String,
    pub arguments: String,
}
