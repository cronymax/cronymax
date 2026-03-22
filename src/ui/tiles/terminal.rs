//! Terminal pane widget — `TerminalPane` struct + rendering.

use super::*;
use crate::ui::widget::{Fragment, Widget};

/// Minimal stateful widget for raw PTY terminal panes.
///
/// Persists across frames in `PaneWidgetStore::terminal`.
#[derive(Debug)]
pub struct TerminalPane {
    pub session_id: SessionId,
}

impl TerminalPane {
    pub fn new(session_id: SessionId) -> Self {
        Self { session_id }
    }
}

/// Temporary view adapting `TerminalPane` to `Widget<egui::Ui>` for raw PTY panes.
///
/// Handles `Pane::Terminal` only — registers the wgpu viewport rect and
/// focus tracking. Chat rendering lives in `ChatPaneView` / `ChatPane` (chat.rs).
pub struct TerminalPaneView<'w> {
    pub widget: &'w mut TerminalPane,
}

impl Widget<egui::Ui> for TerminalPaneView<'_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let ui = &mut *f.painter;
        let dirties = &mut *f.dirties;
        let root_sid = self.widget.session_id;
        let full_rect = ui.available_rect_before_wrap();

        // Track clicks to route keyboard input to the correct split pane.
        if ui.input(|i| i.pointer.any_pressed())
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| full_rect.contains(p))
        {
            f.ui_state.focused_terminal_session = Some(root_sid);
        }

        // Register the full pane area as a wgpu terminal viewport.
        let _r = ui.allocate_rect(full_rect, egui::Sense::click_and_drag());
        dirties.tile_rects.push(TileRect::Terminal {
            session_id: root_sid,
            rect: full_rect,
        });

        // ── DnD split overlay ──
        let is_tab_dragging = egui::DragAndDrop::has_payload_of_type::<TabDragPayload>(ui.ctx());
        if is_tab_dragging {
            let dnd_response = ui.interact(
                full_rect,
                egui::Id::new("pane_dnd").with(root_sid),
                egui::Sense::drag(),
            );
            handle_pane_dnd(ui, root_sid, full_rect, &dnd_response, &mut dirties.actions);
        }
    }
}
