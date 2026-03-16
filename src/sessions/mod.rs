//! Session data models — typed conversation instances derived from a [`Profile`].
//!
//! Each session represents a single conversation/workspace within a profile:
//! - [`chat::ChatSession`] — LLM chat conversation state
//! - [`browser::BrowserSession`] — browser tab with cookies/localStorage
//! - [`terminal::TerminalSession`] — terminal PTY session state
//! - [`channel::ChannelSession`] — messaging channel conversation
//!
//! All sessions share a common [`Session`] base with profile linkage and lifecycle timestamps.

pub mod browser;
pub mod channel;
pub mod chat;
pub mod terminal;

use serde::{Deserialize, Serialize};

// Re-export session types at the `sessions::` level.
pub use browser::BrowserSession;
pub use channel::ChannelSession;
pub use chat::ChatSession;
pub use terminal::TerminalSessionData;

/// The kind of session (discriminant for [`Session`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionType {
    Chat,
    Browser,
    Terminal,
    Channel,
}

/// Common session base — every session is tied to exactly one profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier (UUID).
    pub id: String,
    /// Profile this session belongs to.
    pub profile_id: String,
    /// Session kind discriminant.
    pub session_type: SessionType,
    /// Human-readable session title.
    pub title: String,
    /// Unix timestamp (ms) when the session was created.
    pub created_at: u64,
    /// Unix timestamp (ms) of the last activity.
    pub updated_at: u64,
}

impl Session {
    /// Create a new session with the current timestamp.
    pub fn new(profile_id: &str, session_type: SessionType, title: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: profile_id.to_string(),
            session_type,
            title: title.to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Touch the session — update `updated_at` to now.
    pub fn touch(&mut self) {
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }
}
