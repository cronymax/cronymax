//! Always-On Memory Agent — background LLM-based memory extraction.
//!
//! Inspired by DeerFlow's MemoryMiddleware, this module runs a lightweight
//! background LLM call after each conversation turn to extract persistent
//! facts, preferences, and context from the conversation.
//!
//! # Design
//!
//! The memory agent is NOT in the critical path — it runs asynchronously
//! after the main LLM response has been delivered to the user:
//!
//! ```text
//! User ← response    (immediate)
//!      ↓
//! MemoryAgent.extract(messages)    (background, uses cheap model)
//!      ↓
//! MemoryStore.insert(facts)        (deduped via normalize_whitespace)
//! ```
//!
//! ## Key Principles
//!
//! 1. **Non-blocking**: Never delays the user-facing response.
//! 2. **Cheap model**: Uses `gpt-4o-mini` or similar for extraction.
//! 3. **Debounced**: Only runs when enough new content has accumulated.
//! 4. **Deduped**: Extracted facts pass through `MemoryStore::insert()` which
//!    performs whitespace-normalized deduplication.
//! 5. **Profile-scoped**: Facts are stored per-profile, available across sessions.
#![allow(dead_code)]

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ai::context::{ChatMessage, MessageRole};
use crate::services::memory::{MemoryEntry, MemoryStore, MemoryTag};

// ─── Configuration ───────────────────────────────────────────────────────────

/// Configuration for the always-on memory agent.
#[derive(Debug, Clone)]
pub struct MemoryAgentConfig {
    /// Model to use for memory extraction (cheap/fast model).
    pub model: String,
    /// Minimum new messages since last extraction before triggering.
    pub debounce_messages: usize,
    /// Maximum messages to include in the extraction prompt.
    pub max_context_messages: usize,
    /// Whether the memory agent is enabled.
    pub enabled: bool,
}

impl Default for MemoryAgentConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4o-mini".into(),
            debounce_messages: 4,
            max_context_messages: 20,
            enabled: true,
        }
    }
}

// ─── Extraction Prompt ───────────────────────────────────────────────────────

/// System prompt for the memory extraction LLM call.
const MEMORY_EXTRACTION_PROMPT: &str = r#"You are a memory extraction agent. Your job is to analyze a conversation and extract important facts, preferences, and context that should be remembered for future conversations.

Rules:
1. Extract only FACTUAL information — names, preferences, project details, technical choices, workflow patterns.
2. Each fact should be a single, self-contained sentence.
3. Do NOT extract conversational filler, greetings, or ephemeral task details.
4. Do NOT extract information that is already present in the provided existing memories.
5. Categorize each fact as one of: general, project, preference, fact, instruction, context.
6. Return a JSON array of objects with "content" and "tag" fields.
7. If there is nothing worth remembering, return an empty array: []

Example output:
```json
[
  {"content": "User prefers Rust for backend development", "tag": "preference"},
  {"content": "Project uses SQLite for local storage", "tag": "project"},
  {"content": "Always run cargo check before committing", "tag": "instruction"}
]
```"#;

// ─── Memory Agent ────────────────────────────────────────────────────────────

/// Always-on memory agent that extracts facts from conversations.
pub struct MemoryAgent {
    config: MemoryAgentConfig,
    /// Messages seen since last extraction (debounce counter).
    messages_since_extraction: Arc<Mutex<usize>>,
}

impl MemoryAgent {
    pub fn new(config: MemoryAgentConfig) -> Self {
        Self {
            config,
            messages_since_extraction: Arc::new(Mutex::new(0)),
        }
    }

    /// Access the agent configuration.
    pub fn config(&self) -> &MemoryAgentConfig {
        &self.config
    }

    /// Notify the agent that new messages have been added to the conversation.
    /// Returns true if extraction should be triggered.
    pub async fn notify_new_messages(&self, count: usize) -> bool {
        let mut counter = self.messages_since_extraction.lock().await;
        *counter += count;
        *counter >= self.config.debounce_messages
    }

    /// Reset the debounce counter after extraction.
    pub async fn reset_counter(&self) {
        let mut counter = self.messages_since_extraction.lock().await;
        *counter = 0;
    }

    /// Build the extraction prompt from recent conversation messages
    /// and existing memory entries.
    pub fn build_extraction_messages(
        &self,
        conversation: &[ChatMessage],
        existing_memories: &[MemoryEntry],
    ) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        // System prompt.
        messages.push(ChatMessage::new(
            MessageRole::System,
            MEMORY_EXTRACTION_PROMPT.to_string(),
            crate::ai::context::MessageImportance::System,
            0,
        ));

        // Include existing memories so the LLM avoids duplicates.
        if !existing_memories.is_empty() {
            let existing_text: String = existing_memories
                .iter()
                .map(|e| {
                    format!(
                        "- [{}] {}",
                        format!("{:?}", e.tag).to_lowercase(),
                        e.content
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(ChatMessage::new(
                MessageRole::User,
                format!(
                    "Existing memories (do NOT re-extract these):\n{}",
                    existing_text
                ),
                crate::ai::context::MessageImportance::Normal,
                0,
            ));
        }

        // Include recent conversation (last N messages).
        let recent: Vec<&ChatMessage> = conversation
            .iter()
            .filter(|m| m.role != MessageRole::System && m.role != MessageRole::Info)
            .rev()
            .take(self.config.max_context_messages)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let conversation_text: String = recent
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::User => "User",
                    MessageRole::Assistant => "Assistant",
                    MessageRole::Tool => "Tool",
                    _ => "System",
                };
                format!("{}: {}", role, &m.content[..m.content.len().min(500)])
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        messages.push(ChatMessage::new(
            MessageRole::User,
            format!(
                "Extract memorable facts from this conversation:\n\n{}",
                conversation_text
            ),
            crate::ai::context::MessageImportance::Normal,
            0,
        ));

        messages
    }

    /// Parse the extraction response (JSON array of {content, tag} objects).
    pub fn parse_extraction_response(response: &str) -> Vec<(String, MemoryTag)> {
        // Try to extract JSON from the response (may be wrapped in markdown code blocks).
        let json_str = if let Some(start) = response.find('[') {
            if let Some(end) = response.rfind(']') {
                &response[start..=end]
            } else {
                return Vec::new();
            }
        } else {
            return Vec::new();
        };

        let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(json_str);
        match parsed {
            Ok(items) => items
                .into_iter()
                .filter_map(|item| {
                    let content = item.get("content")?.as_str()?.to_string();
                    if content.is_empty() {
                        return None;
                    }
                    let tag = match item
                        .get("tag")
                        .and_then(|t| t.as_str())
                        .unwrap_or("general")
                    {
                        "project" => MemoryTag::Project,
                        "preference" => MemoryTag::Preference,
                        "fact" => MemoryTag::Fact,
                        "instruction" => MemoryTag::Instruction,
                        "context" => MemoryTag::Context,
                        _ => MemoryTag::General,
                    };
                    Some((content, tag))
                })
                .collect(),
            Err(e) => {
                log::warn!("[MemoryAgent] Failed to parse extraction response: {}", e);
                Vec::new()
            }
        }
    }

    /// Convert extracted facts into `MemoryEntry` objects ready for insertion.
    pub fn facts_to_entries(facts: &[(String, MemoryTag)]) -> Vec<MemoryEntry> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        facts
            .iter()
            .map(|(content, tag)| MemoryEntry {
                id: uuid::Uuid::new_v4().to_string(),
                content: content.clone(),
                tag: tag.clone(),
                pinned: false,
                token_count: content.chars().count().div_ceil(4),
                created_at: now,
                last_used_at: now,
                access_count: 0,
            })
            .collect()
    }
}

// ─── Memory Injection (middleware integration) ───────────────────────────────

/// Build a memory context block suitable for injection into the system prompt.
///
/// Returns a formatted string of memory entries, or `None` if no entries exist
/// or the memory store is disabled.
pub fn render_memory_for_injection(memory: &MemoryStore, max_tokens: usize) -> Option<String> {
    if memory.entries.is_empty() {
        return None;
    }
    let rendered = memory.render_for_prompt(max_tokens);
    if rendered.is_empty() {
        return None;
    }
    Some(format!(
        "\n\n<persistent_memory>\n{}\n</persistent_memory>",
        rendered
    ))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_extraction_response() {
        let response = r#"```json
[
  {"content": "User prefers Rust", "tag": "preference"},
  {"content": "Project uses SQLite", "tag": "project"}
]
```"#;
        let facts = MemoryAgent::parse_extraction_response(response);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].0, "User prefers Rust");
        assert_eq!(facts[0].1, MemoryTag::Preference);
        assert_eq!(facts[1].0, "Project uses SQLite");
        assert_eq!(facts[1].1, MemoryTag::Project);
    }

    #[test]
    fn parse_empty_extraction_response() {
        let response = "[]";
        let facts = MemoryAgent::parse_extraction_response(response);
        assert!(facts.is_empty());
    }

    #[test]
    fn parse_malformed_response() {
        let response = "Sorry, I can't extract anything useful.";
        let facts = MemoryAgent::parse_extraction_response(response);
        assert!(facts.is_empty());
    }

    #[test]
    fn parse_raw_json_response() {
        let response = r#"[{"content": "Uses vim keybindings", "tag": "preference"}]"#;
        let facts = MemoryAgent::parse_extraction_response(response);
        assert_eq!(facts.len(), 1);
    }

    #[test]
    fn parse_filters_empty_content() {
        let response =
            r#"[{"content": "", "tag": "fact"}, {"content": "Real fact", "tag": "fact"}]"#;
        let facts = MemoryAgent::parse_extraction_response(response);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].0, "Real fact");
    }

    #[test]
    fn facts_to_entries_creates_valid_entries() {
        let facts = vec![
            ("User uses macOS".to_string(), MemoryTag::Fact),
            (
                "Always format with rustfmt".to_string(),
                MemoryTag::Instruction,
            ),
        ];
        let entries = MemoryAgent::facts_to_entries(&facts);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tag, MemoryTag::Fact);
        assert_eq!(entries[1].tag, MemoryTag::Instruction);
        assert!(!entries[0].id.is_empty());
        assert!(entries[0].created_at > 0);
    }

    #[tokio::test]
    async fn debounce_triggers_after_threshold() {
        let agent = MemoryAgent::new(MemoryAgentConfig {
            debounce_messages: 3,
            ..Default::default()
        });

        assert!(!agent.notify_new_messages(1).await); // 1 < 3
        assert!(!agent.notify_new_messages(1).await); // 2 < 3
        assert!(agent.notify_new_messages(1).await); // 3 >= 3

        agent.reset_counter().await;
        assert!(!agent.notify_new_messages(1).await); // reset to 1
    }

    #[test]
    fn build_extraction_messages_includes_context() {
        let agent = MemoryAgent::new(MemoryAgentConfig::default());

        let conversation = vec![
            ChatMessage::new(
                MessageRole::System,
                "You are helpful.".into(),
                crate::ai::context::MessageImportance::System,
                0,
            ),
            ChatMessage::new(
                MessageRole::User,
                "I prefer dark themes".into(),
                crate::ai::context::MessageImportance::Normal,
                0,
            ),
            ChatMessage::new(
                MessageRole::Assistant,
                "Got it, noted!".into(),
                crate::ai::context::MessageImportance::Normal,
                0,
            ),
        ];

        let existing = vec![MemoryEntry {
            id: "1".into(),
            content: "User uses macOS".into(),
            tag: MemoryTag::Fact,
            pinned: false,
            token_count: 5,
            created_at: 0,
            last_used_at: 0,
            access_count: 0,
        }];

        let messages = agent.build_extraction_messages(&conversation, &existing);
        assert_eq!(messages.len(), 3); // system + existing memories + conversation
        assert!(messages[0].content.contains("memory extraction agent"));
        assert!(messages[1].content.contains("User uses macOS"));
        assert!(messages[2].content.contains("dark themes"));
    }

    #[test]
    fn render_memory_for_injection_formats_correctly() {
        let mut store = MemoryStore::new("test");
        store.insert(MemoryEntry {
            id: "1".into(),
            content: "User prefers Rust".into(),
            tag: MemoryTag::Preference,
            pinned: false,
            token_count: 5,
            created_at: 0,
            last_used_at: 0,
            access_count: 0,
        });

        let result = render_memory_for_injection(&store, 1000);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("<persistent_memory>"));
        assert!(text.contains("User prefers Rust"));
    }

    #[test]
    fn render_memory_for_injection_returns_none_when_empty() {
        let store = MemoryStore::new("test");
        assert!(render_memory_for_injection(&store, 1000).is_none());
    }
}
