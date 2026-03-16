//! Cross-session services — profile-scoped data that spans multiple sessions.
//!
//! These services provide shared data backends used by all session types
//! within a profile:
//!
//! - [`memory`] — LLM memory & context (long-term memory entries, RAG retrieval)
//! - [`schedule`] — Scheduled task definitions and execution records
//! - [`block`] — Conversation block persistence (chat prompts, responses, tool calls, terminal output)
//!
//! All services are scoped by profile ID. Sessions reference services through
//! their parent profile.

pub mod block;
pub mod memory;
pub mod schedule;
pub mod secret;

// Re-export key types at the `services::` level.
pub use block::{Block, BlockType};
pub use memory::{MemoryConfig, MemoryEntry, MemoryStore, MemoryTag};
pub use schedule::{ExecutionRecord, ScheduledTask, ScheduledTaskStore};
