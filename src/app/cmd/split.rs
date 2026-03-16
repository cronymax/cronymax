//! Terminal split-pane commands extracted from cmd_webview.rs

use crate::app::*;

pub(super) fn handle_split(state: &mut AppState, action: &UiAction) {
    let (dir, insert_after) = match action {
        UiAction::SplitLeft => (egui_tiles::LinearDir::Horizontal, false),
        UiAction::SplitRight => (egui_tiles::LinearDir::Horizontal, true),
        _ => (egui_tiles::LinearDir::Vertical, true), // SplitDown
    };
    if let Some(active_sid) = tiles::active_terminal_session(&state.tile_tree)
        && let Some(active_tile) = tiles::find_terminal_tile(&state.tile_tree, active_sid)
    {
        let id = state.next_id;
        state.next_id += 1;
        let shell = state
            .config
            .terminal
            .shell
            .clone()
            .unwrap_or_else(crate::renderer::platform::default_shell);
        let phys = state.window.inner_size();
        let logical = phys.to_logical::<f32>(state.window.scale_factor());
        let (_, cols, rows) = ui::compute_single_pane(
            logical.width as u32,
            logical.height as u32,
            &state.renderer.cell_size,
            &state.styles,
        );
        let sandbox = active_sandbox_policy(state);
        let session = TerminalSession::new(
            id,
            &shell,
            cols,
            rows,
            state.config.terminal.scrollback_lines,
            Some(&sandbox),
            Some(state.proxy.clone()),
        );
        state.sessions.insert(id, session);
        // Inherit the mode from the active tab so Terminal→Terminal
        // and Chat→Chat.
        let is_active_terminal = state
            .prompt_editors
            .get(&active_sid)
            .is_some_and(|pe| !pe.visible);
        let mut prompt_editor = PromptState::new();
        prompt_editor.visible = !is_active_terminal;
        state.prompt_editors.insert(id, prompt_editor);
        // Split panes do NOT get a tab entry — they share the
        // parent tab container's single tab bar.
        let new_pane = if is_active_terminal {
            tiles::Pane::Terminal {
                session_id: id,
                title: "cronymax".to_string(),
            }
        } else {
            tiles::Pane::Chat {
                session_id: id,
                title: "cronymax".to_string(),
            }
        };
        tiles::split_pane_dir(
            &mut state.tile_tree,
            active_tile,
            new_pane,
            dir,
            insert_after,
        );
        log::info!(
            "UI split {:?} insert_after={}: new session {}",
            dir,
            insert_after,
            id
        );
        state.scheduler.mark_dirty();
    }
}
