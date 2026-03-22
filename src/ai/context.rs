// Context window management — token counting, message history, pruning.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Role of a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
    /// Display-only informational message — not sent to LLM, not persisted.
    /// Used for pull progress, status notifications, etc.
    Info,
}

/// Importance level determines pruning priority (higher = harder to remove).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageImportance {
    /// Throwaway context — pruned first.
    Ephemeral = 0,
    /// Normal conversation — pruned after ephemeral.
    Normal = 1,
    /// User-pinned or compaction summary — never pruned by sliding window.
    Pinned = 2,
    /// System prompt — never pruned.
    System = 3,
    /// User-starred pane block — survives compaction, never pruned.
    Starred = 4,
}

/// A single message in the chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: u32,
    pub role: MessageRole,
    pub content: String,
    pub importance: MessageImportance,
    pub token_count: u32,
    pub timestamp_ms: u64,
    /// For tool messages, the ID of the tool call this responds to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// For assistant messages that invoked tool calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<crate::ai::stream::ToolCallInfo>,
    /// Links this message to its corresponding Block::Stream cell_id.
    /// Used for thread branching (forking conversation from a block).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_id: Option<u32>,
}

impl ChatMessage {
    /// Create a new message with auto-generated timestamp.
    pub fn new(
        role: MessageRole,
        content: String,
        importance: MessageImportance,
        token_count: u32,
    ) -> Self {
        Self {
            id: 0, // Set by MessageHistory::push
            role,
            content,
            importance,
            token_count,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            tool_call_id: None,
            tool_calls: Vec::new(),
            cell_id: None,
        }
    }
}

/// Token counter using tiktoken for accurate token estimation.
pub struct TokenCounter {
    /// Fallback: chars / 4 heuristic for unknown models.
    _heuristic_divisor: usize,
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenCounter {
    pub fn new() -> Self {
        Self {
            _heuristic_divisor: 4,
        }
    }

    /// Count tokens in a text string. Uses heuristic (chars/4) as fallback.
    pub fn count(&self, text: &str, _model: &str) -> usize {
        // tiktoken-rs: try to load BPE for the model.
        // For now, use a character-based heuristic that's reasonable for most models.
        // Proper tiktoken integration requires matching model names to BPE encodings.
        let char_count = text.chars().count();
        // ~4 chars per token is a reasonable approximation for English text.
        char_count.div_ceil(4)
    }
}

/// Ordered message history with token budget management.
pub struct MessageHistory {
    messages: VecDeque<ChatMessage>,
    next_id: u32,
    /// Maximum tokens for the entire context window.
    pub max_context_tokens: usize,
    /// Tokens reserved for completion output.
    pub reserve_tokens: usize,
}

impl MessageHistory {
    pub fn new(max_context_tokens: usize, reserve_tokens: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            next_id: 1,
            max_context_tokens,
            reserve_tokens,
        }
    }

    /// Available tokens = max - reserve - system message tokens.
    pub fn available_tokens(&self) -> usize {
        let system_tokens: usize = self
            .messages
            .iter()
            .filter(|m| m.importance == MessageImportance::System)
            .map(|m| m.token_count as usize)
            .sum();
        self.max_context_tokens
            .saturating_sub(self.reserve_tokens)
            .saturating_sub(system_tokens)
    }

    /// Total tokens across all messages.
    pub fn total_tokens(&self) -> usize {
        self.messages.iter().map(|m| m.token_count as usize).sum()
    }

    /// Push a new message, assigning an auto-incrementing ID.
    pub fn push(&mut self, mut msg: ChatMessage) {
        msg.id = self.next_id;
        self.next_id += 1;
        self.messages.push_back(msg);
    }

    /// Remove lowest-importance messages from oldest first until within budget.
    /// Never removes Pinned, System, or Starred messages.
    pub fn sliding_window_drop(&mut self) {
        let budget = self.available_tokens();
        // Remove Ephemeral first, then Normal. Never remove Pinned, System, or Starred.
        for importance in [MessageImportance::Ephemeral, MessageImportance::Normal] {
            while self.total_tokens() > budget {
                // Find the oldest message with this importance level.
                if let Some(idx) = self
                    .messages
                    .iter()
                    .position(|m| m.importance == importance)
                {
                    self.messages.remove(idx);
                } else {
                    break;
                }
            }
        }
    }

    /// Recount all tokens and prune to budget. Returns number of messages removed.
    pub fn prune_to_budget(&mut self, counter: &TokenCounter, model: &str) -> usize {
        // Recount token counts.
        for msg in self.messages.iter_mut() {
            msg.token_count = counter.count(&msg.content, model) as u32;
        }
        let before = self.messages.len();
        self.sliding_window_drop();
        before - self.messages.len()
    }

    /// Get messages in chronological order for API calls (System first).
    /// Filters out `Info` messages (display-only, never sent to LLM).
    pub fn for_api(&self) -> Vec<ChatMessage> {
        let mut result: Vec<ChatMessage> = Vec::with_capacity(self.messages.len());
        // System messages first (skip Info).
        for msg in &self.messages {
            if msg.role == MessageRole::System {
                result.push(msg.clone());
            }
        }
        // Then all non-system in order (skip Info).
        for msg in &self.messages {
            if msg.role != MessageRole::System && msg.role != MessageRole::Info {
                result.push(msg.clone());
            }
        }
        result
    }

    /// Get all messages for persistence.
    pub fn to_persistent(&self) -> Vec<ChatMessage> {
        self.messages.iter().cloned().collect()
    }

    /// Clear all messages except System, Pinned, and Starred.
    pub fn clear_non_essential(&mut self) {
        self.messages.retain(|m| {
            m.importance == MessageImportance::System
                || m.importance == MessageImportance::Pinned
                || m.importance == MessageImportance::Starred
        });
    }

    /// Get the number of messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Find a message by its ID.
    pub fn get_by_id(&self, id: u32) -> Option<&ChatMessage> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Remove a message by its ID. Returns true if removed.
    pub fn remove_by_id(&mut self, id: u32) -> bool {
        if let Some(idx) = self.messages.iter().position(|m| m.id == id) {
            self.messages.remove(idx);
            true
        } else {
            false
        }
    }

    /// Fork the history up to and including messages tagged with the given cell_id.
    ///
    /// Copies all messages where `cell_id` is `None` (system prompts) or
    /// `cell_id <= branch_cell_id`. Returns a new `MessageHistory` with
    /// the same token budget and reset internal ID counter.
    pub fn fork_up_to_cell(
        &self,
        branch_cell_id: u32,
        max_context_tokens: usize,
        reserve_tokens: usize,
    ) -> MessageHistory {
        let mut forked = MessageHistory::new(max_context_tokens, reserve_tokens);
        for msg in &self.messages {
            let include = match msg.cell_id {
                None => true, // System prompts, pinned summaries
                Some(cid) => cid <= branch_cell_id,
            };
            if include {
                forked.push(msg.clone());
            }
        }
        forked
    }
}

// ─── Snapshot types (read-only views for AI skills) ─────────────────────────

/// Read-only snapshot of a session's message history for AI skill access.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageHistorySnapshot {
    pub session_id: u32,
    pub messages: Vec<ChatMessageInfo>,
    pub token_count: u32,
    pub max_tokens: u32,
}

/// Lightweight message info for skill-level inspection.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatMessageInfo {
    pub id: u32,
    pub role: MessageRole,
    /// First 200 characters of content.
    pub content_preview: String,
    pub importance: MessageImportance,
    pub starred: bool,
    pub token_count: u32,
    pub timestamp_ms: u64,
}
