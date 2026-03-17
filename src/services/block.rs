// Block service — universal conversation block for DB persistence.
//
// A Block represents a single conversation element that can be cached to a database.
// It provides a unified type across all session kinds (chat, terminal, channel, scheduler).
//
// Examples:
// - Chat prompt / LLM model / response / tool calls
// - Terminal command / output
// - Schedule task execution details
// - Informational messages

use serde::{Deserialize, Serialize};

/// A persisted conversation block — the universal unit of conversation history.
///
/// Blocks are stored in the database and can be replayed, starred, compacted,
/// or exported. Each block belongs to a session (which belongs to a profile).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Unique block identifier.
    pub id: String,
    /// Session this block belongs to.
    pub session_id: String,
    /// Profile this block belongs to (denormalized for efficient queries).
    pub profile_id: String,
    /// Block type discriminant and payload.
    pub block_type: BlockType,
    /// Whether this block is starred (survives compaction).
    #[serde(default)]
    pub starred: bool,
    /// Token count for this block's content.
    #[serde(default)]
    pub token_count: u32,
    /// Unix timestamp (ms) when the block was created.
    pub created_at: u64,
    /// Unix timestamp (ms) of the last update.
    pub updated_at: u64,
}

/// Block type discriminant — typed payload for each kind of conversation element.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockType {
    /// User chat prompt sent to the LLM.
    ChatPrompt {
        content: String,
        #[serde(default)]
        model: Option<String>,
    },
    /// LLM response (assistant message).
    ChatResponse {
        content: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        prompt_tokens: u32,
        #[serde(default)]
        completion_tokens: u32,
    },
    /// Tool invocation within a chat session.
    ToolCall {
        name: String,
        arguments: String,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    /// Terminal command execution.
    TerminalCommand {
        command: String,
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        exit_code: Option<i32>,
    },
    /// Scheduled task execution record.
    ScheduleExecution {
        task_id: String,
        task_name: String,
        status: String,
        #[serde(default)]
        output: String,
        #[serde(default)]
        duration_ms: u64,
    },
    /// Informational message (system notice, status update).
    Info { text: String },
    /// Channel message (incoming or outgoing).
    ChannelMessage {
        sender: String,
        content: String,
        #[serde(default)]
        is_outgoing: bool,
    },
}

impl Block {
    /// Create a new block with the current timestamp.
    pub fn new(session_id: &str, profile_id: &str, block_type: BlockType) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            profile_id: profile_id.to_string(),
            block_type,
            starred: false,
            token_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Star this block (protects from compaction).
    pub fn star(&mut self) {
        self.starred = true;
    }

    /// Unstar this block.
    pub fn unstar(&mut self) {
        self.starred = false;
    }

    /// Get a human-readable summary of this block's content.
    pub fn summary(&self) -> String {
        match &self.block_type {
            BlockType::ChatPrompt { content, .. } => {
                format!("? {}", content.chars().take(80).collect::<String>())
            }
            BlockType::ChatResponse { content, .. } => content.chars().take(80).collect::<String>(),
            BlockType::ToolCall { name, .. } => format!("Tool: {}", name),
            BlockType::TerminalCommand { command, .. } => {
                format!("$ {}", command.chars().take(80).collect::<String>())
            }
            BlockType::ScheduleExecution {
                task_name, status, ..
            } => {
                format!("Schedule: {} ({})", task_name, status)
            }
            BlockType::Info { text } => text.chars().take(80).collect::<String>(),
            BlockType::ChannelMessage {
                sender, content, ..
            } => {
                format!(
                    "{}: {}",
                    sender,
                    content.chars().take(60).collect::<String>()
                )
            }
        }
    }
}
