//! Post-frame action processing

use crate::app::*;

pub(super) fn process_post_frame(
    state: &mut AppState,
    event_loop: &ActiveEventLoop,
    _pane_rects: &[tiles::TileRect],
    ui_actions: Vec<UiAction>,
    submitted_cmds: Vec<(u32, String)>,
) {
    // Clear dirty for ALL terminal sessions — including
    // background tabs — so stale flags never cause perpetual
    // scheduler.mark_dirty() calls in about_to_wait.
    for session in state.sessions.values_mut() {
        session.is_dirty = false;
    }

    // Process egui UI actions (captured from draw_all inside egui.run closure).
    for action in ui_actions {
        handle_ui_action(state, action, event_loop);
    }

    // Process submitted input line commands (per-pane).
    for (sid, cmd) in submitted_cmds {
        if cmd.starts_with(':') {
            handle_colon_command(state, &cmd, event_loop);
        } else if let Some(stripped) = cmd.strip_prefix('$') {
            // Script/command mode: strip `$` prefix and send to PTY.
            let shell_cmd = stripped.trim().to_string();
            if !shell_cmd.is_empty()
                && let Some(session) = state.sessions.get_mut(&sid)
            {
                // Freeze the previous live terminal cell (if any).
                freeze_last_live_terminal_with_session(&mut state.prompt_editors, sid, session);

                let payload = format!("{}\n", shell_cmd);
                session.write_to_pty(payload.as_bytes());

                // Record a CommandBlock and a Block.
                if let Some(prompt_editor) = state.prompt_editors.get_mut(&sid)
                    && prompt_editor.visible
                {
                    let abs_row = session.state.abs_cursor_row();
                    let block_id = prompt_editor.command_blocks.len();
                    let prompt = prompt_editor.prefix.clone();
                    prompt_editor.command_blocks.push(CommandBlock {
                        id: block_id,
                        prompt,
                        cmd: shell_cmd,
                        abs_row,
                        filter_text: String::new(),
                        filter_open: false,
                    });
                    prompt_editor.blocks.push(BlockMode::Terminal {
                        block_id,
                        frozen_output: None,
                    });
                }
            }
        } else {
            // Default: chat mode — route to per-session LLM chat.
            let chat_text = cmd.trim().to_string();
            if !chat_text.is_empty() {
                // Freeze the previous live terminal cell (if any).
                freeze_last_live_terminal(state, sid);

                // Create a BlockMode::Stream entry.
                if let Some(prompt_editor) = state.prompt_editors.get_mut(&sid) {
                    let cell_id = prompt_editor.next_chat_cell_id;
                    prompt_editor.next_chat_cell_id += 1;
                    prompt_editor.blocks.push(BlockMode::Stream {
                        id: cell_id,
                        prompt: chat_text.clone(),
                        response: String::new(),
                        is_streaming: true,
                        tool_status: None,
                        tool_calls_log: Vec::new(),
                    });
                    // Pre-create a CommonMark cache for this cell.
                    if let Some(chat) = state.session_chats.get_mut(&sid) {
                        chat.cell_caches
                            .insert(cell_id, egui_commonmark::CommonMarkCache::default());
                    }
                }

                submit_chat(state, sid, &chat_text);
            }
        }
    }
}
