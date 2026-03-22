//! Frame rendering — thin wrapper that delegates to `ui::draw`.
//!
//! Pre-draw (PTY processing, session cleanup, sync) runs at `AppState` level,
//! then the actual rendering is dispatched to `Ui::draw_frame`.  Post-frame
//! command processing stays here because `submit_chat` / `:ollama` need full
//! `AppState` access.

mod post;

use crate::ui::sync::sync_ui_state;

use super::*;

// ─── Frame rendering ─────────────────────────────────────────────────────────

pub(super) fn handle_redraw(state: &mut AppState, event_loop: &ActiveEventLoop) {
    // ── Pre-draw: PTY processing, session cleanup ────────────
    let mut any_exited = Vec::new();
    for (id, session) in state.sessions.iter_mut() {
        session.process_pty_output();
        if session.exited {
            any_exited.push(*id);
        }
    }

    // Sync per-session CWD into prompt editors (for file picker).
    for (id, session) in &state.sessions {
        if let Some(ref cwd) = session.cwd
            && let Some(pe) = state.ui_state.prompt_editors.get_mut(id)
            && pe.cwd.as_deref() != Some(cwd)
        {
            pe.cwd = Some(cwd.clone());
        }
    }

    for id in &any_exited {
        state.sessions.remove(id);
        tiles::remove_terminal_pane(&mut state.ui.tile_tree, *id);
        state.ui_state.prompt_editors.remove(id);
        log::info!("Session {} exited", id);
    }
    // Sync tab removal in ui_state
    state.ui_state.tabs.retain(|t| match t {
        TabInfo::Chat { session_id, .. } | TabInfo::Terminal { session_id, .. } => {
            state.sessions.contains_key(session_id)
        }
        _ => true,
    });
    if state.ui_state.active_tab >= state.ui_state.tabs.len() {
        state.ui_state.active_tab = state.ui_state.tabs.len().saturating_sub(1);
    }

    if state.sessions.is_empty() {
        log::info!("All sessions exited, closing window");
        event_loop.exit();
        return;
    }

    let active_sid = match tiles::active_terminal_session(&state.ui.tile_tree) {
        Some(id) => id,
        None => {
            event_loop.exit();
            return;
        }
    };

    // Sync UiState from app state.
    sync_ui_state(state, active_sid);

    // ── Draw frame (split borrow) ────────────────────────────
    let draw_result = {
        let (ui, mut ctx) = state.split_ui();
        ui.draw_frame(&mut ctx, event_loop)
    };

    // ── Post-frame processing ────────────────────────────────
    if let Some(result) = draw_result {
        post::process_post_frame(state, event_loop, result.actions, result.commands);
    }
}
