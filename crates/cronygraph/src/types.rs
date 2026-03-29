//! Core types — messages, tokens, tool calls, skill handlers.
//!
//! These are the foundational data types used throughout the agent graph framework.
//! They are intentionally simple, serializable, and framework-agnostic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

// ─── Message Types ───────────────────────────────────────────────────────────

/// Role of a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
    /// Display-only informational message — not sent to LLM, not persisted.
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
    pub tool_calls: Vec<ToolCallInfo>,
    /// Application-specific cell/block identifier (optional).
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
            id: 0,
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

// ─── Token Usage ─────────────────────────────────────────────────────────────

/// Token usage reported by the LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// ─── Tool Call Info ──────────────────────────────────────────────────────────

/// A single tool call from the LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    /// Provider-assigned tool call ID.
    pub id: String,
    /// Name of the function to invoke.
    pub function_name: String,
    /// JSON-encoded arguments.
    pub arguments: String,
}

// ─── Skill Types ─────────────────────────────────────────────────────────────

/// Async handler for a skill invocation.
pub type SkillHandler = Arc<
    dyn Fn(
            serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<serde_json::Value>> + Send>>
        + Send
        + Sync,
>;

/// A tool skill definition (name, description, parameter JSON schema, category).
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    /// Skill category for filtering (e.g., "general", "terminal", "browser").
    pub category: String,
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_new_sets_defaults() {
        let msg = ChatMessage::new(
            MessageRole::User,
            "hello".into(),
            MessageImportance::Normal,
            2,
        );
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "hello");
        assert!(msg.tool_call_id.is_none());
        assert!(msg.tool_calls.is_empty());
        assert!(msg.cell_id.is_none());
        assert!(msg.timestamp_ms > 0);
    }

    #[test]
    fn message_importance_ordering() {
        assert!(MessageImportance::Ephemeral < MessageImportance::Normal);
        assert!(MessageImportance::Normal < MessageImportance::Pinned);
        assert!(MessageImportance::Pinned < MessageImportance::System);
        assert!(MessageImportance::System < MessageImportance::Starred);
    }
}
