// Chat panel UI — per-session inline chat cells with streaming markdown.

use std::collections::{HashMap, HashSet};

use egui_commonmark::CommonMarkCache;

use crate::ai::client::ModelSelection;
use crate::ai::context::{
    ChatMessage, MessageHistory, MessageImportance, MessageRole, TokenCounter,
};

// ─── Per-Session Chat State ──────────────────────────────────────────────────

/// Per-session chat state: messages, streaming buffer, token budget, LLM handle.
pub struct SessionChat {
    /// Optional per-session model override (set via the Compbox).
    pub model_override: Option<ModelSelection>,
    /// Whether the LLM is currently streaming.
    pub is_streaming: bool,
    /// Buffer accumulating streamed tokens for the current response.
    pub streaming_buffer: String,
    /// Completed chat messages for display (user + assistant pairs).
    pub messages: Vec<ChatMessage>,
    /// Per-message CommonMark caches (keyed by message ID).
    pub message_caches: HashMap<u32, CommonMarkCache>,
    /// Cache for the in-progress streaming content.
    pub streaming_cache: CommonMarkCache,
    /// Whether to auto-scroll the chat area to bottom on next frame.
    pub scroll_to_bottom: bool,
    /// Message history with token budget management (sent to LLM API).
    pub history: MessageHistory,
    /// LLM stream session ID (monotonically increasing, for correlating events).
    pub llm_session_id: u32,
    /// Active LLM streaming task handle (for cancellation).
    pub active_stream: Option<tokio::task::JoinHandle<()>>,
    /// Total tokens used in current context (display only).
    pub tokens_used: u32,
    /// Maximum tokens for the context window (display only).
    #[allow(dead_code)]
    pub tokens_limit: u32,
    /// Per-cell CommonMark caches (keyed by Block::Chat id).
    pub cell_caches: HashMap<u32, CommonMarkCache>,
    /// Persistent UUID for cross-session identity (survives app restarts).
    pub persistent_id: Option<String>,

    // ── Thread (branching) fields ─────────────────────────────────
    /// If this is a thread, the session ID of the parent it was branched from.
    pub parent_session_id: Option<crate::renderer::terminal::SessionId>,
    /// If this is a thread, the cell_id of the block it was branched from.
    pub branch_cell_id: Option<u32>,
    /// Map from block cell_id → child thread session ID.
    pub threads: HashMap<u32, crate::renderer::terminal::SessionId>,
    /// Number of tool-call rounds in the current agentic loop.
    pub tool_rounds: u32,
    /// Maximum tool-call rounds before auto-stopping (0 = unlimited).
    pub max_tool_rounds: u32,
    /// In-memory cache of starred block cell IDs (synced on toggle).
    pub starred_ids: HashSet<u32>,
    /// Pinned markdown content displayed as a single block at the top of the pane
    /// (used by the History view and similar info-only tabs).
    pub pinned_content: Option<String>,
}

impl SessionChat {
    /// Create a new per-session chat state.
    pub fn new(max_context_tokens: usize, reserve_tokens: usize) -> Self {
        Self {
            model_override: None,
            is_streaming: false,
            streaming_buffer: String::new(),
            messages: Vec::new(),
            message_caches: HashMap::new(),
            streaming_cache: CommonMarkCache::default(),
            scroll_to_bottom: false,
            history: MessageHistory::new(max_context_tokens, reserve_tokens),
            llm_session_id: 0,
            active_stream: None,
            tokens_used: 0,
            tokens_limit: max_context_tokens as u32,
            cell_caches: HashMap::new(),
            persistent_id: Some(uuid::Uuid::new_v4().to_string()),
            parent_session_id: None,
            branch_cell_id: None,
            threads: HashMap::new(),
            tool_rounds: 0,
            max_tool_rounds: 10,
            starred_ids: HashSet::new(),
            pinned_content: None,
        }
    }

    /// Append a streamed token to the buffer.
    pub fn append_token(&mut self, token: &str) {
        self.streaming_buffer.push_str(token);
        self.scroll_to_bottom = true;
    }

    /// Finalize the streaming response — move buffer to messages.
    pub fn finalize_streaming(&mut self, msg: ChatMessage) {
        let id = msg.id;
        self.messages.push(msg);
        self.message_caches.insert(id, CommonMarkCache::default());
        self.streaming_buffer.clear();
        self.streaming_cache = CommonMarkCache::default();
        self.is_streaming = false;
        self.scroll_to_bottom = true;
    }

    /// Add a completed message (e.g., user message).
    pub fn add_message(&mut self, msg: ChatMessage) {
        let id = msg.id;
        self.messages.push(msg);
        self.message_caches.insert(id, CommonMarkCache::default());
        self.scroll_to_bottom = true;
    }

    /// Submit a user chat prompt. Adds to history and messages.
    /// Returns the messages to send to the LLM API.
    /// `cell_id` links the message to its Block::Stream block for thread branching.
    pub fn submit_user_message(
        &mut self,
        text: &str,
        token_counter: &TokenCounter,
        model: &str,
        cell_id: Option<u32>,
    ) -> Vec<ChatMessage> {
        let tc = token_counter.count(text, model) as u32;
        let mut user_msg = ChatMessage::new(
            MessageRole::User,
            text.to_string(),
            MessageImportance::Normal,
            tc,
        );
        user_msg.cell_id = cell_id;
        self.history.push(user_msg.clone());
        self.add_message(user_msg);
        self.is_streaming = true;
        self.llm_session_id += 1;
        self.history.for_api()
    }

    /// Push an LLM system prompt into the history.
    pub fn set_system_prompt(&mut self, prompt: &str, token_counter: &TokenCounter, model: &str) {
        let tc = token_counter.count(prompt, model) as u32;
        self.history.push(ChatMessage::new(
            MessageRole::System,
            prompt.to_string(),
            MessageImportance::System,
            tc,
        ));
    }

    /// Whether this session has any chat content to display.
    #[allow(dead_code)]
    pub fn has_content(&self) -> bool {
        !self.messages.is_empty() || self.is_streaming
    }

    /// Clear all chat state (new session).
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.messages.clear();
        self.message_caches.clear();
        self.streaming_buffer.clear();
        self.streaming_cache = CommonMarkCache::default();
        self.is_streaming = false;
        self.tokens_used = 0;
        self.cell_caches.clear();
    }

    /// Add a display-only info message (not sent to LLM, not persisted).
    pub fn add_info_message(&mut self, text: &str) {
        let msg = ChatMessage::new(
            MessageRole::Info,
            text.to_string(),
            MessageImportance::Ephemeral,
            0,
        );
        self.add_message(msg);
    }

    /// Update the last info message in-place (for progress reporting).
    /// If the last message is not an info message, appends a new one.
    pub fn update_last_info_message(&mut self, text: &str) {
        if let Some(last) = self.messages.last_mut()
            && last.role == MessageRole::Info
        {
            last.content = text.to_string();
            // Invalidate the cache for this message.
            self.message_caches.remove(&last.id);
            self.message_caches
                .insert(last.id, CommonMarkCache::default());
            self.scroll_to_bottom = true;
            return;
        }
        self.add_info_message(text);
    }
}

// NOTE: Inline chat cell rendering has been moved to tiles.rs
// (render_chat_cell_inline) as part of the unified cell layout.
// SessionChat is still used for LLM conversation history management.
