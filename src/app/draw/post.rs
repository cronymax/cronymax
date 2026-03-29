//! Post-frame action processing

use crate::app::*;

pub(super) fn process_post_frame(
    state: &mut AppState,
    event_loop: &ActiveEventLoop,
    ui_actions: Vec<UiAction>,
    submitted_cmds: Vec<(u32, String)>,
    colon_commands: Vec<String>,
) {
    // Clear dirty for ALL terminal sessions — including
    // background tabs — so stale flags never cause perpetual
    // scheduler.mark_dirty() calls in about_to_wait.
    for session in state.sessions.values_mut() {
        session.is_dirty = false;
    }

    // Process egui UI actions (captured from draw_all inside egui.run closure).
    for action in ui_actions {
        state.dispatch_ui_action(action, event_loop);
    }

    // Process colon commands from the command palette.
    for cmd in colon_commands {
        state.dispatch_colon_command(&format!(":{}", cmd), event_loop);
    }

    // Process submitted input line commands (per-pane).
    for (sid, cmd) in submitted_cmds {
        if cmd.starts_with(':') {
            state.dispatch_colon_command(&cmd, event_loop);
        } else if let Some(stripped) = cmd.strip_prefix('$') {
            // Script/command mode: strip `$` prefix and send to PTY.
            let shell_cmd = stripped.trim().to_string();
            if !shell_cmd.is_empty() {
                // For threads, route to the parent session's PTY.
                let pty_sid = state
                    .session_chats
                    .get(&sid)
                    .and_then(|c| c.parent_session_id)
                    .unwrap_or(sid);

                if let Some(session) = state.sessions.get_mut(&pty_sid) {
                    // Freeze the previous live terminal cell (if any).
                    freeze_last_live_terminal_with_session(
                        &mut state.ui_state.prompt_editors,
                        sid,
                        session,
                    );

                    let payload = format!("{}\n", shell_cmd);
                    session.write_to_pty(payload.as_bytes());

                    // Record a CommandBlock and a Block.
                    if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&sid)
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
                        prompt_editor.blocks.push(Block::Terminal {
                            block_id,
                            frozen_output: None,
                        });
                    }
                }
            }
        } else {
            // Default: chat mode — route to per-session LLM chat.
            let chat_text = cmd.trim().to_string();
            if !chat_text.is_empty() {
                // Freeze the previous live terminal cell (if any).
                freeze_last_live_terminal(state, sid);

                // Create a Block::Stream entry.
                let current_cell_id =
                    if let Some(prompt_editor) = state.ui_state.prompt_editors.get_mut(&sid) {
                        let cell_id = prompt_editor.next_chat_cell_id;
                        prompt_editor.next_chat_cell_id += 1;
                        prompt_editor.blocks.push(Block::Stream {
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
                        Some(cell_id)
                    } else {
                        None
                    };

                submit_chat(state, sid, &chat_text, current_cell_id);
            }
        }
    }
}

/// Inner implementation that takes the session by reference to avoid borrow conflicts.
pub(super) fn freeze_last_live_terminal_with_session(
    prompt_editors: &mut HashMap<SessionId, PromptState>,
    sid: SessionId,
    session: &TerminalSession,
) {
    let il = match prompt_editors.get_mut(&sid) {
        Some(il) => il,
        None => return,
    };

    // Find the last block; if it's a live terminal, freeze its output.
    if let Some(Block::Terminal {
        block_id,
        frozen_output,
    }) = il.blocks.last_mut()
        && frozen_output.is_none()
    {
        // Determine the row range for this command block.
        let abs_start = il
            .command_blocks
            .get(*block_id)
            .map(|b| b.abs_row)
            .unwrap_or(0);
        let abs_end = session.state.abs_cursor_row();
        // Capture non-empty output (skip the prompt row itself).
        let text = session.state.capture_text(abs_start + 1, abs_end);
        *frozen_output = Some(text);
    }
}

// ─── Cell Freeze Helpers ─────────────────────────────────────────────────────

/// Freeze the last live terminal cell for a session, capturing its text output.
/// Called before creating a new cell (chat or terminal) to ensure chronological ordering.
pub(super) fn freeze_last_live_terminal(state: &mut AppState, sid: SessionId) {
    // For threads, route to the parent session's PTY.
    let pty_sid = state
        .session_chats
        .get(&sid)
        .and_then(|c| c.parent_session_id)
        .unwrap_or(sid);
    if let Some(session) = state.sessions.get(&pty_sid) {
        freeze_last_live_terminal_with_session(&mut state.ui_state.prompt_editors, sid, session);
    }
}
