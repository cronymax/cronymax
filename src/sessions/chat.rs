// Chat session — LLM conversation data model derived from Session.
//
// Manages the pure data state of an LLM chat conversation:
// - Message history (user/assistant/system/tool messages)
// - Model selection and token budget
// - References to cross-session services (Memory, Block, ScheduleTask)

use serde::{Deserialize, Serialize};

use super::{Session, SessionType};

/// Chat session data model — the persistent, serializable state of an LLM conversation.
///
/// This is the **data model** layer. UI-specific state (CommonMarkCache, scroll position,
/// streaming buffer) lives in [`crate::ui::chat::SessionChat`], which wraps this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    /// Base session (profile linkage, timestamps, ID).
    pub session: Session,
    /// LLM provider name (e.g. "openai", "copilot", "ollama").
    #[serde(default)]
    pub provider: String,
    /// Model identifier (e.g. "gpt-4o", "claude-3-opus").
    #[serde(default)]
    pub model: String,
    /// System prompt override for this session.
    #[serde(default)]
    pub system_prompt: String,
    /// Total tokens consumed in this conversation.
    #[serde(default)]
    pub token_count: u32,
    /// Maximum context window size (tokens).
    #[serde(default)]
    pub token_limit: u32,
    /// Whether this session is currently active.
    #[serde(default)]
    pub active: bool,
}

impl ChatSession {
    /// Create a new chat session for the given profile.
    pub fn new(profile_id: &str, title: &str) -> Self {
        Self {
            session: Session::new(profile_id, SessionType::Chat, title),
            provider: String::new(),
            model: String::new(),
            system_prompt: String::new(),
            token_count: 0,
            token_limit: 0,
            active: true,
        }
    }

    /// Convenience: get the session ID.
    pub fn id(&self) -> &str {
        &self.session.id
    }

    /// Convenience: get the profile ID.
    pub fn profile_id(&self) -> &str {
        &self.session.profile_id
    }
}
