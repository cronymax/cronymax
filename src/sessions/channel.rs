// Channel session — messaging channel data model derived from Session.
//
// Manages the persistent state of a messaging channel conversation:
// - Channel platform identity (Lark, Telegram, etc.)
// - Connection state and message counters
// - References to per-channel Memory / Block / ScheduleTask

use serde::{Deserialize, Serialize};

use super::{Session, SessionType};

/// Channel session data model — persistent state for a messaging channel conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSession {
    /// Base session (profile linkage, timestamps, ID).
    pub session: Session,
    /// Channel platform identifier (e.g. "lark", "telegram").
    pub channel_type: String,
    /// Channel instance ID (maps to channel config).
    pub channel_id: String,
    /// Display name of the channel.
    #[serde(default)]
    pub channel_name: String,
    /// Number of messages received.
    #[serde(default)]
    pub messages_received: u64,
    /// Number of messages sent.
    #[serde(default)]
    pub messages_sent: u64,
    /// Whether the channel is currently connected.
    #[serde(default)]
    pub connected: bool,
}

impl ChannelSession {
    /// Create a new channel session.
    pub fn new(profile_id: &str, channel_type: &str, channel_id: &str, name: &str) -> Self {
        Self {
            session: Session::new(profile_id, SessionType::Channel, name),
            channel_type: channel_type.to_string(),
            channel_id: channel_id.to_string(),
            channel_name: name.to_string(),
            messages_received: 0,
            messages_sent: 0,
            connected: false,
        }
    }

    /// Record a received message.
    pub fn on_message_received(&mut self) {
        self.messages_received += 1;
        self.session.touch();
    }

    /// Record a sent message.
    pub fn on_message_sent(&mut self) {
        self.messages_sent += 1;
        self.session.touch();
    }
}
