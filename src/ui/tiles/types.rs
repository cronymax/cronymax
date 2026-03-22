//! Type definitions for the tile/pane system.

use super::*;

// ─── Pane Widget Store ───────────────────────────────────────────────────────

use super::browser::BrowserViewPane;
use super::channel::ChannelPane;
use super::chat::ChatPane;
use super::terminal::TerminalPane;

#[derive(Debug, Clone)]
pub struct TabDragPayload {
    pub session_id: SessionId,
}

/// Direction to dock a dragged tab relative to the drop target pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockDirection {
    Left,
    Right,
    Top,
    Bottom,
}

// ─── Pane Type ───────────────────────────────────────────────────────────────

/// Content of a leaf tile in the tiling tree — see widget hierarchy §2.2.
///
/// Variants map to [`PaneKind`](super::types::PaneKind):
/// - `Chat` → Chat pane (§2.2.1) — egui prompt + PTY output grid
/// - `Terminal` → Terminal pane (§2.2.2) — raw PTY, no prompt editor
/// - `BrowserView` → Docked browser view pane (§2.2.3)
/// - `Channel` → Channel conversation pane (§2.2.4)
#[derive(Debug, Clone)]
pub enum Pane {
    /// Chat pane (§2.2.1) — egui prompt + PTY output grid.
    Chat {
        session_id: SessionId,
        title: String,
    },
    /// Terminal pane (§2.2.2) — raw PTY, no prompt editor.
    Terminal {
        session_id: SessionId,
        title: String,
    },
    BrowserView {
        webview_id: u32,
        title: String,
        url: String,
    },
    /// A channel conversation tab (e.g. Lark group, Slack channel).
    Channel {
        channel_id: String,
        channel_name: String,
    },
}

impl PartialEq for Pane {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Pane::Chat { session_id: a, .. }, Pane::Chat { session_id: b, .. }) => a == b,
            (Pane::Terminal { session_id: a, .. }, Pane::Terminal { session_id: b, .. }) => a == b,
            (Pane::BrowserView { webview_id: a, .. }, Pane::BrowserView { webview_id: b, .. }) => {
                a == b
            }
            (Pane::Channel { channel_id: a, .. }, Pane::Channel { channel_id: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// Physical pixel rect for a tile pane — either a wgpu terminal or a native webview.
#[derive(Debug, Clone)]
pub enum TileRect {
    /// Terminal pane; used for wgpu viewport and PTY resize.
    Terminal {
        session_id: SessionId,
        rect: egui::Rect,
    },
    /// Browser view pane; used to position the native webview.
    BrowserView { webview_id: u32, rect: egui::Rect },
}

/// Persistent store for all pane widget instances.
///
/// Lives in `TilesPanel`. Passed into `Behavior` as `&mut` each frame.
/// Widgets are created lazily on first access and destroyed on pane close.
#[derive(Default, Debug)]
pub struct PaneWidgetStore {
    pub chat: std::collections::HashMap<SessionId, ChatPane>,
    pub terminal: std::collections::HashMap<SessionId, TerminalPane>,
    pub browser: std::collections::HashMap<u32, BrowserViewPane>,
    pub channel: std::collections::HashMap<String, ChannelPane>,
}

impl PaneWidgetStore {
    /// Get or create a chat widget for a session.
    pub fn chat_widget(&mut self, sid: SessionId) -> &mut ChatPane {
        self.chat
            .entry(sid)
            .or_insert_with(|| ChatPane::new(sid))
    }

    /// Get or create a terminal widget for a session.
    pub fn terminal_widget(&mut self, sid: SessionId) -> &mut TerminalPane {
        self.terminal
            .entry(sid)
            .or_insert_with(|| TerminalPane::new(sid))
    }

    /// Get or create a browser widget for a webview.
    pub fn browser_widget(&mut self, wid: u32) -> &mut BrowserViewPane {
        self.browser
            .entry(wid)
            .or_insert_with(|| BrowserViewPane::new(wid))
    }

    /// Get or create a channel widget.
    pub fn channel_widget(&mut self, id: String, name: String) -> &mut ChannelPane {
        self.channel
            .entry(id.clone())
            .or_insert_with(|| ChannelPane::new(id, name))
    }

    /// Clean up widgets for a closed terminal/chat pane.
    pub fn remove_terminal(&mut self, sid: SessionId) {
        self.chat.remove(&sid);
        self.terminal.remove(&sid);
    }

    /// Clean up widget for a closed browser pane.
    pub fn remove_browser(&mut self, wid: u32) {
        self.browser.remove(&wid);
    }

    /// Clean up widget for a closed channel pane.
    pub fn remove_channel(&mut self, id: &str) {
        self.channel.remove(id);
    }
}

/// Intermediate layout for a terminal pane (content area + input area).
#[derive(Clone, Copy, Debug)]
pub struct TerminalLayout {
    pub content_rect: egui::Rect,
    pub input_rect: egui::Rect,
    pub has_input: bool,
    pub is_chat_mode: bool,
    /// Chat context info: (tokens_used, tokens_limit). Only set in chat mode.
    pub chat_context: Option<(u32, u32)>,
}
