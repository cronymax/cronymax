//! Tiles widget (§2) — egui_tiles integration for the pane tree.
//!
//! Implements the tiling layout with:
//! - **Tabs bar** (§2.1): dynamic-width tab strip with right-side actions
//! - **Pane tree** (§2.2): [`Pane`] enum with Chat, Terminal, BrowserView, Channel variants
//!
//! See [`crate::ui::types::PaneKind`] for the semantic pane classification.

mod behavior;
mod browser;
mod channel;
mod overlays;
mod terminal;
mod tree;
mod types;

use std::collections::HashMap;

use crate::channels::{ChannelDisplayMessage, ConnectionState};
use crate::renderer::terminal::SessionId;
use crate::ui::UiAction;
use crate::ui::block::Block;
use crate::ui::chat::SessionChat;
use crate::ui::i18n::t;
use crate::ui::icons::{self, Icon};
use crate::ui::prompt::PromptState;
use crate::ui::styles::Styles;
use crate::ui::tiles::behavior::Behavior;
use crate::ui::types::CommandEntry;
use crate::ui::types::TooltipRequest;
use crate::ui::widget::{Fragment, Widget};

use overlays::*;
pub use tree::*;
pub use types::*;

/// Alias for `egui_tiles::LinearDir` — the direction of a tile split.
///
/// Re-exported through `ui::tiles` so that `app/` code can reference
/// split directions without importing `egui_tiles` directly.
pub type SplitDir = egui_tiles::LinearDir;

/// Mutable per-frame state passed as `&mut` (exclusive access).
///
/// Contains all mutable data that pane widgets may read or modify
/// during rendering. Sequential widget calls re-borrow.
pub struct FrameState<'a> {
    /// Per-session prompt editor state.
    pub prompt_editors: &'a mut HashMap<SessionId, PromptState>,
    /// Per-session LLM chat state (token counts, model info).
    pub session_chats: &'a mut HashMap<SessionId, SessionChat>,
    /// Terminal/stream content blocks (owned per frame).
    pub blocks: HashMap<SessionId, Block>,
    /// Live streaming output per session (owned per frame).
    pub live_outputs: HashMap<SessionId, String>,
    /// Channel message lists (read-only reference).
    pub channel_messages: &'a HashMap<String, Vec<ChannelDisplayMessage>>,
    /// Lark channel connection status.
    pub channel_connection_state: ConnectionState,
    /// Command suggestions for prompt auto-complete.
    pub commands: Vec<CommandEntry>,
}

/// Tiles panel widget — wraps the egui_tiles tree + Behavior pattern.
pub struct TilesPanel<'a> {
    pub tile_tree: &'a mut egui_tiles::Tree<Pane>,
    pub blocks: std::collections::HashMap<SessionId, Block>,
    pub session_chats: &'a mut std::collections::HashMap<SessionId, crate::ui::chat::SessionChat>,
    pub live_outputs: std::collections::HashMap<SessionId, String>,
    pub channel_messages:
        &'a std::collections::HashMap<String, Vec<crate::channels::ChannelDisplayMessage>>,
    pub channel_connection_state: crate::channels::ConnectionState,
}

impl Widget for TilesPanel<'_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        let ctx = f.ctx();
        let colors = std::rc::Rc::clone(&f.colors);
        let styles = f.styles;
        let ui_state = &mut *f.ui_state;
        let dirties = &mut *f.dirties;
        let pinned_set: std::collections::HashSet<u32> =
            ui_state.pinned_tabs.iter().copied().collect();

        // Extract values from ui_state before moving it into FrameState.
        let commands = ui_state.command_suggestions.commands.clone();
        let pinned_tabs = ui_state.pinned_tabs.clone();
        let address_bar_editing = ui_state.address_bar.editing;
        let visible_tab_count = ui_state
            .tabs
            .iter()
            .filter(|t| !pinned_set.contains(&t.id()))
            .count()
            .max(1);

        // Take fields out of ui_state to enable split borrows:
        // FrameState needs &mut prompt_editors, Behavior needs &mut pane_widgets,
        // and Fragment::new needs &mut ui_state — all at the same time.
        let mut prompt_editors = std::mem::take(&mut ui_state.prompt_editors);
        let mut pane_widgets = std::mem::take(&mut ui_state.pane_widgets);

        let frame_state = FrameState {
            prompt_editors: &mut prompt_editors,
            session_chats: self.session_chats,
            blocks: std::mem::take(&mut self.blocks),
            live_outputs: std::mem::take(&mut self.live_outputs),
            channel_messages: self.channel_messages,
            channel_connection_state: self.channel_connection_state,
            commands,
        };

        let mut behavior = Behavior {
            fragment: Fragment::new(ctx, colors, styles, ui_state, dirties),
            state: frame_state,
            widgets: &mut pane_widgets,
            address_bar_editing,
            pinned_tabs,
            tab_bar_width: ctx.screen_rect().width(),
            clicked_terminal_session: None,
            pending_star_toggle: None,
            tooltip: None,
            visible_tab_count,
        };

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                self.tile_tree.ui(&mut behavior, ui);
            });

        // Sync state back.
        behavior.fragment.ui_state.address_bar.editing = behavior.address_bar_editing;
        if let Some(sid) = behavior.clicked_terminal_session {
            behavior.fragment.ui_state.focused_terminal_session = Some(sid);
        }
        if let Some((session_id, message_id)) = behavior.pending_star_toggle {
            behavior
                .fragment
                .dirties
                .actions
                .push(UiAction::ToggleStarred {
                    session_id,
                    message_id,
                });
        }

        // Store tooltip so the orchestrator can route it to FloatPanel.
        behavior.fragment.ui_state.docked_tooltip = behavior.tooltip;

        // Swap taken fields back into ui_state via the still-live borrows.
        std::mem::swap(behavior.state.prompt_editors, &mut behavior.fragment.ui_state.prompt_editors);
        std::mem::swap(behavior.widgets, &mut behavior.fragment.ui_state.pane_widgets);
    }
}
