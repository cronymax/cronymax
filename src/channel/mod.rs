//! Channel subsystem — trait-based messaging platform integrations.
//!
//! This module defines the [`Channel`] trait, [`ChannelMessage`], [`ChannelManager`],
//! and related types. The channel subsystem enables bidirectional messaging with
//! external platforms (e.g., Feishu/Lark, Slack, Telegram) routed through the
//! AI agent pipeline.
//!
//! # Architecture
//!
//! - **Transport concern**: Channels live alongside `src/ai/`, not inside it.
//! - **Feature-gated**: All functionality is behind `ClawConfig.enabled`.
//! - **Async**: All I/O runs on the tokio runtime via `AppState.runtime`.
//! - **Platform-agnostic core**: [`ChannelManager`], [`agent_loop`], and
//!   [`memory`] contain **zero** platform-specific references. Only
//!   [`register_channels()`] and the individual channel modules know about
//!   specific platforms.
//!
//! # Adding a New Channel (e.g., Telegram)
//!
//! Adding support for a new messaging platform requires exactly **3 touches**:
//!
//! 1. **Create the channel module** — `src/channel/telegram.rs`
//!    - Implement the [`Channel`] trait (connect, disconnect, send_message, etc.)
//!    - Handle platform-specific auth, WebSocket/polling, and message parsing
//!
//! 2. **Add a config variant** — in `src/channel/config.rs`
//!    ```rust,ignore
//!    #[derive(Debug, Clone, Serialize, Deserialize)]
//!    pub struct TelegramChannelConfig {
//!        pub bot_token: String,
//!        pub allowed_users: Vec<String>,
//!        #[serde(default)]
//!        pub profile_id: String,
//!    }
//!
//!    // In ChannelConfig enum:
//!    pub enum ChannelConfig {
//!        Lark(LarkChannelConfig),
//!        Telegram(TelegramChannelConfig),  // ← add this
//!    }
//!    ```
//!
//! 3. **Register in `register_channels()`** — add a match arm in this file
//!    ```rust,ignore
//!    ChannelConfig::Telegram(tg_cfg) => {
//!        let channel = telegram::TelegramChannel::new(tg_cfg.clone());
//!        if let Err(e) = manager.register(Box::new(channel)).await {
//!            log::error!("Failed to register Telegram channel: {}", e);
//!        }
//!    }
//!    ```
//!
//! **No changes needed in**: [`ChannelManager`], [`agent_loop::process_message()`],
//! [`memory::ChannelMemoryStore`], UI rendering, or any dispatch logic.
//!
//! # Error Classification
//!
//! | Error Category  | Examples                         | Channel Responsibility          |
//! |-----------------|----------------------------------|---------------------------------|
//! | **Transient**   | Network timeout, rate limit      | Retry with exponential backoff  |
//! | **Auth**        | Expired token, revoked API key   | Refresh token / re-authenticate |
//! | **Fatal**       | Invalid credentials, deleted bot | Return error, set state=`Error` |
//! | **Message**     | Malformed payload, unknown type  | Log + skip, don't disconnect    |
//! | **Validation**  | Missing required config fields   | Caught in `validate()`, skipped |
//!
//! # Extensibility Audit (SC-007)
//!
//! The following modules are verified platform-agnostic (zero Lark/platform references):
//! - `channel::agent_loop` — 6-stage pipeline operates on [`ChannelMessage`] only
//! - `channel::memory` — stores/recalls by channel_id string, no platform logic
//! - `ChannelManager` — registers/routes via the [`Channel`] trait, no platform code
//! - `channel::config::ClawConfig` — Vec<ChannelConfig> with serde tagged enum
//!
//! Only `register_channels()` and `pub mod lark` (module declaration) reference
//! specific platform implementations, by design.
#![allow(dead_code)]

pub mod agent_loop;
pub mod config;
pub mod lark;
pub mod memory;

use std::collections::HashMap;

use winit::event_loop::EventLoopProxy;

use crate::ai::stream::AppEvent;
use crate::channel::config::{ChannelConfig, ClawConfig};

// ─── Core Types ──────────────────────────────────────────────────────────────

/// Result of a single bot configuration check.
#[derive(Debug, Clone)]
pub struct BotCheckResult {
    /// Check category label (e.g., "Authentication", "Bot Info").
    pub label: String,
    /// Whether this check passed.
    pub passed: bool,
    /// Human-readable detail or error message.
    pub detail: String,
}

/// A single message displayed in a channel conversation tab.
#[derive(Debug, Clone)]
pub struct ChannelDisplayMessage {
    /// Sender name (or ID if name unavailable).
    pub sender: String,
    /// Message text content.
    pub content: String,
    /// Whether this message was sent by the bot (outgoing) or received (incoming).
    pub is_outgoing: bool,
    /// Unix timestamp in milliseconds.
    pub timestamp: i64,
}

/// Normalized message representation flowing between channels and the AI pipeline.
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    /// Unique message ID from the source platform (e.g., Lark `message_id`).
    pub id: String,
    /// Identifier of the source channel (e.g., `"lark"`).
    pub channel_id: String,
    /// Platform-specific sender ID (e.g., Lark `open_id`).
    pub sender_id: String,
    /// Human-readable sender name (if available).
    pub sender_name: Option<String>,
    /// Platform-specific chat/conversation ID (for reply routing).
    pub chat_id: String,
    /// Plain text content of the message.
    pub content: String,
    /// Unix timestamp (milliseconds) when the message was created.
    pub timestamp: i64,
    /// Routing information for sending a reply back through the channel.
    pub reply_target: ReplyTarget,
}

/// Opaque routing information for sending a reply back through a channel.
#[derive(Debug, Clone)]
pub struct ReplyTarget {
    /// Which channel to route the reply through.
    pub channel_id: String,
    /// Platform-specific chat/conversation ID.
    pub chat_id: String,
    /// Original message ID (for threaded replies, if supported).
    pub message_id: Option<String>,
}

/// Runtime status of a channel connection.
#[derive(Debug, Clone)]
pub struct ChannelStatus {
    /// Channel identifier (e.g., `"lark"`).
    pub channel_id: String,
    /// Current connection state.
    pub state: ConnectionState,
    /// Most recent error message, if any.
    pub last_error: Option<String>,
    /// Unix timestamp (ms) when connection was established.
    pub connected_since: Option<i64>,
    /// Count of messages received since connection.
    pub messages_received: u64,
    /// Count of messages sent since connection.
    pub messages_sent: u64,
}

/// Connection state for a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected, not attempting.
    Disconnected,
    /// Connection in progress.
    Connecting,
    /// Active and healthy.
    Connected,
    /// Lost connection, attempting to restore.
    Reconnecting,
    /// Failed, not retrying (e.g., invalid credentials).
    Error,
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Connected => write!(f, "Connected"),
            Self::Reconnecting => write!(f, "Reconnecting"),
            Self::Error => write!(f, "Error"),
        }
    }
}

// ─── Channel Trait ───────────────────────────────────────────────────────────

/// A messaging channel integration.
///
/// Implementors handle platform-specific connection management,
/// authentication, message receiving (via event loop), and message sending.
///
/// # Lifecycle
///
/// 1. `connect()` — Establish connection to the messaging platform
/// 2. Event loop runs (platform-specific), emitting [`ChannelMessage`] via callback
/// 3. `send_message()` — Send replies back through the platform
/// 4. `disconnect()` — Gracefully close the connection
///
/// # Error Handling
///
/// All methods return `anyhow::Result`. Transient errors (network, rate limit)
/// should be retried internally with backoff. Fatal errors (invalid credentials)
/// should be returned immediately.
///
/// # Implementor Requirements
///
/// - `connect()` must be **idempotent** — calling on an already-connected channel is a no-op.
/// - `disconnect()` must be **idempotent** — calling on a disconnected channel is a no-op.
/// - `on_message` callback must only be invoked for **authorized senders** — call
///   `is_sender_authorized()` internally before invoking.
/// - **Reconnection** is the channel's responsibility — retry with exponential backoff
///   (1s, 2s, 4s, 8s, max 60s). `status()` should reflect `Reconnecting`.
/// - **Token refresh** is the channel's responsibility — refresh proactively before expiry.
///
/// # Example
///
/// ```rust,ignore
/// struct MyChannel { /* ... */ }
///
/// #[async_trait::async_trait]
/// impl Channel for MyChannel {
///     fn id(&self) -> &str { "my_platform" }
///     fn display_name(&self) -> &str { "My Platform" }
///     async fn connect(&mut self, on_message: Box<dyn Fn(ChannelMessage) + Send + Sync>) -> anyhow::Result<()> { /* ... */ Ok(()) }
///     async fn disconnect(&mut self) -> anyhow::Result<()> { /* ... */ Ok(()) }
///     async fn send_message(&self, target: &ReplyTarget, content: &str) -> anyhow::Result<()> { /* ... */ Ok(()) }
///     fn status(&self) -> ChannelStatus { /* ... */ todo!() }
///     fn is_sender_authorized(&self, sender_id: &str) -> bool { /* ... */ true }
/// }
/// ```
#[async_trait::async_trait]
pub trait Channel: Send + Sync {
    /// Unique identifier for this channel type (e.g., `"lark"`, `"telegram"`).
    fn id(&self) -> &str;

    /// Human-readable display name (e.g., `"Feishu/Lark"`).
    fn display_name(&self) -> &str;

    /// Establish connection to the messaging platform.
    ///
    /// Starts the channel's event loop (e.g., WebSocket connection).
    /// Incoming messages are delivered via the `on_message` callback.
    /// Returns when the connection is established (event loop runs in background).
    async fn connect(
        &mut self,
        on_message: Box<dyn Fn(ChannelMessage) + Send + Sync>,
    ) -> anyhow::Result<()>;

    /// Gracefully disconnect from the messaging platform.
    ///
    /// Waits for in-flight operations to complete (up to a timeout).
    /// After disconnect, no more messages will be received or sent.
    async fn disconnect(&mut self) -> anyhow::Result<()>;

    /// Send a message through this channel.
    ///
    /// # Arguments
    /// * `target` — Where to send the reply (chat_id, optional message_id for threading)
    /// * `content` — Plain text content to send
    async fn send_message(&self, target: &ReplyTarget, content: &str) -> anyhow::Result<()>;

    /// Current connection status.
    fn status(&self) -> ChannelStatus;

    /// Check if a sender is authorized per the channel's allowlist.
    ///
    /// Returns `true` if the sender is permitted, `false` otherwise.
    /// - Empty allowlist → deny all
    /// - `["*"]` → allow all
    /// - Otherwise → exact match against sender ID
    fn is_sender_authorized(&self, sender_id: &str) -> bool;

    /// Set a callback to be fired when the connection state changes.
    /// Called by ChannelManager before `connect()` to wire up status events.
    /// Default implementation is a no-op.
    fn set_status_callback(&mut self, _cb: Box<dyn Fn(ChannelStatus) + Send + Sync>) {}
}

// ─── ChannelManager ──────────────────────────────────────────────────────────

/// Orchestrates multiple channel instances.
///
/// The `ChannelManager` is responsible for:
/// - Registering and connecting channels when Claw mode is enabled
/// - Routing outbound replies to the correct channel
/// - Shutting down all channels when Claw mode is disabled or the app exits
/// - Tracking channel statuses for UI display
pub struct ChannelManager {
    /// Registered channel instances, keyed by channel ID.
    channels: HashMap<String, Box<dyn Channel>>,
    /// Event loop proxy for sending events to the main thread.
    proxy: EventLoopProxy<AppEvent>,
}

impl ChannelManager {
    /// Create a new manager with a proxy for sending events to the main loop.
    pub fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            channels: HashMap::new(),
            proxy,
        }
    }

    /// Register and connect a channel.
    ///
    /// The channel's event loop starts in the background. Incoming messages
    /// are forwarded to the main event loop via `AppEvent::ChannelMessageReceived`.
    pub async fn register(&mut self, mut channel: Box<dyn Channel>) -> anyhow::Result<()> {
        let channel_id = channel.id().to_string();
        log::info!(
            "Registering channel: {} ({})",
            channel.display_name(),
            channel_id
        );

        // Set up status change callback so the WS loop can notify the UI
        // when connection state changes (Connected, Reconnecting, etc.).
        let proxy_status = self.proxy.clone();
        let status_cb = Box::new(move |status: ChannelStatus| {
            let _ = proxy_status.send_event(AppEvent::ChannelStatusChanged {
                channel_id: status.channel_id.clone(),
                status,
            });
        });
        channel.set_status_callback(status_cb);

        let proxy = self.proxy.clone();
        let on_message = Box::new(move |msg: ChannelMessage| {
            let _ = proxy.send_event(AppEvent::ChannelMessageReceived { message: msg });
        });

        channel.connect(on_message).await?;

        // Notify UI of initial status.
        let status = channel.status();
        let _ = self.proxy.send_event(AppEvent::ChannelStatusChanged {
            channel_id: status.channel_id.clone(),
            status: status.clone(),
        });

        self.channels.insert(channel_id, channel);
        Ok(())
    }

    /// Disconnect and remove all channels.
    ///
    /// Called when Claw mode is disabled or the application exits.
    pub async fn shutdown_all(&mut self) -> anyhow::Result<()> {
        log::info!(
            "Shutting down all channels ({} registered)",
            self.channels.len()
        );
        let mut errors = Vec::new();

        for (id, channel) in self.channels.iter_mut() {
            if let Err(e) = channel.disconnect().await {
                log::error!("Failed to disconnect channel {}: {}", id, e);
                errors.push(e);
            }
        }
        self.channels.clear();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Failed to disconnect {} channel(s)",
                errors.len()
            ))
        }
    }

    /// Send a reply through the appropriate channel.
    pub async fn send_reply(&self, target: &ReplyTarget, content: &str) -> anyhow::Result<()> {
        let channel = self
            .channels
            .get(&target.channel_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown channel: {}", target.channel_id))?;

        channel.send_message(target, content).await
    }

    /// Get status of all registered channels.
    pub fn statuses(&self) -> Vec<ChannelStatus> {
        self.channels.values().map(|ch| ch.status()).collect()
    }
}

// ─── Channel Registration ────────────────────────────────────────────────────

/// Register all channels from the configuration into the manager.
///
/// Iterates `config.channels` and creates a channel instance for each entry.
/// Invalid configurations are logged and skipped (non-fatal).
///
/// # Adding a New Channel
///
/// This is the **only function** that needs a new match arm when adding a
/// platform. The arm instantiates the impl and calls `manager.register()`.
/// Everything else — routing, agent loop, memory, UI — is generic.
///
/// ```rust,ignore
/// ChannelConfig::Telegram(tg_cfg) => {
///     let channel = telegram::TelegramChannel::new(tg_cfg.clone());
///     if let Err(e) = manager.register(Box::new(channel)).await {
///         log::error!("Failed to register Telegram channel: {}", e);
///     }
/// }
/// ```
///
/// # Error Handling
///
/// - **Validation errors**: Logged as warning, channel skipped
/// - **Registration errors**: Logged as error, channel skipped
/// - **All errors are non-fatal**: Other channels continue normally
pub async fn register_channels(
    manager: &mut ChannelManager,
    config: &ClawConfig,
    secret_store: std::sync::Arc<crate::secret::SecretStore>,
) -> anyhow::Result<()> {
    if !config.enabled {
        log::info!("Claw mode disabled, skipping channel registration");
        return Ok(());
    }

    config.validate_unique_ids()?;

    for ch_config in &config.channels {
        if let Err(e) = ch_config.validate() {
            log::warn!(
                "Skipping invalid channel {}: {}",
                ch_config.display_name(),
                e
            );
            continue;
        }

        match ch_config {
            ChannelConfig::Lark(lark_cfg) => {
                let channel = lark::LarkChannel::new(lark_cfg.clone(), secret_store.clone());
                if let Err(e) = manager.register(Box::new(channel)).await {
                    log::error!("Failed to register Lark channel: {}", e);
                }
            }
        }
    }

    log::info!("Registered {} channel(s)", manager.statuses().len());
    Ok(())
}
