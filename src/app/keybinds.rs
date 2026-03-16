//! Sub-module extracted from app/mod.rs

use super::*;

// ─── Keybinding Matcher ──────────────────────────────────────────────────────

pub(super) fn match_keybinding(
    event: &winit::event::KeyEvent,
    modifiers: &ModifiersState,
) -> Option<Action> {
    use winit::keyboard::{Key, NamedKey};

    if event.state != winit::event::ElementState::Pressed {
        return None;
    }

    let ctrl = modifiers.control_key();
    let shift = modifiers.shift_key();
    let super_key = modifiers.super_key();

    if super_key {
        match &event.logical_key {
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("c") => return Some(Action::Copy),
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("v") => {
                return Some(Action::Paste);
            }
            Key::Character(c) if c.as_str() == "," => return Some(Action::ToggleSettings),
            _ => {}
        }
    }

    if ctrl && shift {
        match &event.logical_key {
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("t") => {
                return Some(Action::NewChat);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("w") => {
                return Some(Action::CloseTab);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("c") => return Some(Action::Copy),
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("v") => {
                return Some(Action::Paste);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("d") => {
                return Some(Action::SplitHorizontal);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("p") => {
                return Some(Action::CommandMode);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("f") => {
                return Some(Action::ToggleFilter);
            }
            Key::Named(NamedKey::Tab) => return Some(Action::PrevTab),
            Key::Character(c) if c.as_str() == "=" || c.as_str() == "+" => {
                return Some(Action::FontSizeUp);
            }
            Key::Character(c) if c.as_str() == "-" => return Some(Action::FontSizeDown),
            _ => {}
        }
    }

    if ctrl && !shift {
        match &event.logical_key {
            Key::Named(NamedKey::Tab) => return Some(Action::NextTab),
            Key::Named(NamedKey::PageUp) => return Some(Action::ScrollPageUp),
            Key::Named(NamedKey::PageDown) => return Some(Action::ScrollPageDown),
            Key::Character(c) if c.as_str() == "=" || c.as_str() == "+" => {
                return Some(Action::FontSizeUp);
            }
            Key::Character(c) if c.as_str() == "-" => return Some(Action::FontSizeDown),
            _ => {}
        }
    }

    if shift && !ctrl {
        match &event.logical_key {
            Key::Named(NamedKey::PageUp) => return Some(Action::ScrollPageUp),
            Key::Named(NamedKey::PageDown) => return Some(Action::ScrollPageDown),
            _ => {}
        }
    }

    None
}

// ─── Action Handler ──────────────────────────────────────────────────────────

pub(super) fn handle_action(state: &mut AppState, action: Action) {
    match action {
        Action::NewChat => {
            let id = state.next_id;
            state.next_id += 1;
            let shell = state
                .config
                .terminal
                .shell
                .clone()
                .unwrap_or_else(crate::renderer::platform::default_shell);
            let (_, cols, rows) = ui::compute_single_pane(
                state.window.inner_size().width,
                state.window.inner_size().height,
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
            tiles::add_chat_tab(&mut state.tile_tree, id, "cronymax");

            // Create per-session input line.
            let mut prompt_editor = PromptState::new();
            prompt_editor.visible = true;
            // Copy model list from an existing editor so the ComboBox is populated.
            if let Some(existing) = state.prompt_editors.values().next() {
                prompt_editor.model_items = existing.model_items.clone();
                prompt_editor.selected_model_idx = existing.selected_model_idx;
            }
            state.prompt_editors.insert(id, prompt_editor);

            // Create per-session chat state.
            let (ctx, res) = llm_context_limits(state);
            let mut chat = crate::ui::chat::SessionChat::new(ctx, res);
            if let Some(sp) = state
                .llm_client
                .as_ref()
                .and_then(|c| c.system_prompt().map(String::from))
            {
                let model = llm_model_name(state);
                chat.set_system_prompt(&sp, &state.token_counter, &model);
            }
            state.session_chats.insert(id, chat);

            // Add to ui_state tabs.
            state.ui_state.tabs.push(TabInfo::Chat {
                session_id: id,
                title: "cronymax".into(),
            });
            state.ui_state.active_tab = state.ui_state.tabs.len() - 1;

            log::info!("New tab: session {}", id);
            state.scheduler.mark_dirty();
        }
        Action::NewTerminal => {
            let id = state.next_id;
            state.next_id += 1;
            let shell = state
                .config
                .terminal
                .shell
                .clone()
                .unwrap_or_else(crate::renderer::platform::default_shell);
            let (_, cols, rows) = ui::compute_single_pane(
                state.window.inner_size().width,
                state.window.inner_size().height,
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
            tiles::add_terminal_tab(&mut state.tile_tree, id, "cronymax");

            // Terminal mode: prompt editor hidden.
            let mut prompt_editor = PromptState::new();
            prompt_editor.visible = false;
            state.prompt_editors.insert(id, prompt_editor);

            // Create per-session chat state (still needed for AI interaction).
            let (ctx, res) = llm_context_limits(state);
            let mut chat = crate::ui::chat::SessionChat::new(ctx, res);
            if let Some(sp) = state
                .llm_client
                .as_ref()
                .and_then(|c| c.system_prompt().map(String::from))
            {
                let model = llm_model_name(state);
                chat.set_system_prompt(&sp, &state.token_counter, &model);
            }
            state.session_chats.insert(id, chat);

            // Add to ui_state tabs as terminal.
            state.ui_state.tabs.push(TabInfo::Terminal {
                session_id: id,
                title: "cronymax".into(),
            });
            state.ui_state.active_tab = state.ui_state.tabs.len() - 1;

            log::info!("New terminal tab: session {}", id);
            state.scheduler.mark_dirty();
        }
        Action::CloseTab => {
            if let Some(sid) = tiles::active_terminal_session(&state.tile_tree) {
                state.sessions.remove(&sid);
                tiles::remove_terminal_pane(&mut state.tile_tree, sid);
                state.prompt_editors.remove(&sid);
                // Clean up per-session chat (abort any active stream).
                if let Some(mut chat) = state.session_chats.remove(&sid)
                    && let Some(handle) = chat.active_stream.take()
                {
                    handle.abort();
                }
                state.ui_state.tabs.retain(
                    |t| !matches!(t, TabInfo::Chat { session_id: s, .. } | TabInfo::Terminal { session_id: s, .. } if *s == sid),
                );
                if state.ui_state.active_tab >= state.ui_state.tabs.len() {
                    state.ui_state.active_tab = state.ui_state.tabs.len().saturating_sub(1);
                }
                log::info!("Closed tab: session {}", sid);
                state.scheduler.mark_dirty();
            }
        }
        Action::NextTab => {
            tiles::next_terminal_tab(&mut state.tile_tree);
            state.scheduler.mark_dirty();
        }
        Action::PrevTab => {
            tiles::prev_terminal_tab(&mut state.tile_tree);
            state.scheduler.mark_dirty();
        }
        Action::Copy => {
            if let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                && let Some(session) = state.sessions.get(&sid)
            {
                let term = session.state.term();
                let grid = term.grid();
                let cols = session.grid_size.cols as usize;
                let rows = session.grid_size.rows as usize;
                let mut text_buf = String::with_capacity(cols * rows + rows);
                for row_idx in 0..rows {
                    let line = alacritty_terminal::index::Line(row_idx as i32);
                    for col_idx in 0..cols {
                        let col = alacritty_terminal::index::Column(col_idx);
                        let c = grid[line][col].c;
                        if c.is_control() || c == '\0' {
                            text_buf.push(' ');
                        } else {
                            text_buf.push(c);
                        }
                    }
                    let trimmed = text_buf.trim_end_matches(' ').len();
                    text_buf.truncate(trimmed);
                    if row_idx < rows - 1 {
                        text_buf.push('\n');
                    }
                }
                let trimmed = text_buf.trim_end_matches('\n');
                input::copy_to_clipboard(trimmed);
                log::info!("Copied {} chars to clipboard", trimmed.len());
            }
        }
        Action::Paste => {
            if let Some(text) = input::paste_from_clipboard()
                && let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                && let Some(session) = state.sessions.get_mut(&sid)
            {
                session.write_to_pty(text.as_bytes());
            }
        }
        Action::FontSizeUp => {
            state.config.font.size = (state.config.font.size + 1.0).min(128.0);
            log::info!("Font size: {}", state.config.font.size);
            state.scheduler.mark_dirty();
        }
        Action::FontSizeDown => {
            state.config.font.size = (state.config.font.size - 1.0).max(1.0);
            log::info!("Font size: {}", state.config.font.size);
            state.scheduler.mark_dirty();
        }
        Action::ScrollUp => {
            if let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                && let Some(session) = state.sessions.get_mut(&sid)
            {
                session.state.scroll_up(3);
                state.scheduler.mark_dirty();
            }
        }
        Action::ScrollDown => {
            if let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                && let Some(session) = state.sessions.get_mut(&sid)
            {
                session.state.scroll_down(3);
                state.scheduler.mark_dirty();
            }
        }
        Action::ScrollPageUp => {
            if let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                && let Some(session) = state.sessions.get_mut(&sid)
            {
                session.state.scroll_page_up();
                state.scheduler.mark_dirty();
            }
        }
        Action::ScrollPageDown => {
            if let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                && let Some(session) = state.sessions.get_mut(&sid)
            {
                session.state.scroll_page_down();
                state.scheduler.mark_dirty();
            }
        }
        Action::SplitVertical | Action::SplitHorizontal => {
            let dir = if matches!(action, Action::SplitHorizontal) {
                egui_tiles::LinearDir::Horizontal
            } else {
                egui_tiles::LinearDir::Vertical
            };

            // Find the active tile to split.
            if let Some(active_sid) = tiles::active_terminal_session(&state.tile_tree)
                && let Some(active_tile) = tiles::find_terminal_tile(&state.tile_tree, active_sid)
            {
                // Create a new terminal session for the new pane.
                let id = state.next_id;
                state.next_id += 1;
                let shell = state
                    .config
                    .terminal
                    .shell
                    .clone()
                    .unwrap_or_else(crate::renderer::platform::default_shell);
                let (_, cols, rows) = ui::compute_single_pane(
                    state.window.inner_size().width,
                    state.window.inner_size().height,
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

                // Inherit the mode from the active tab.
                let is_active_terminal = state
                    .prompt_editors
                    .get(&active_sid)
                    .is_some_and(|pe| !pe.visible);
                let mut prompt_editor = PromptState::new();
                prompt_editor.visible = !is_active_terminal;
                state.prompt_editors.insert(id, prompt_editor);

                state.ui_state.tabs.push(if is_active_terminal {
                    TabInfo::Terminal {
                        session_id: id,
                        title: "cronymax".into(),
                    }
                } else {
                    TabInfo::Chat {
                        session_id: id,
                        title: "cronymax".into(),
                    }
                });

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
                tiles::split_pane_dir(&mut state.tile_tree, active_tile, new_pane, dir, true);
                log::info!("Split {:?}: new session {}", dir, id);
                state.scheduler.mark_dirty();
            }
        }
        Action::CommandMode => {
            state.colon_buf = Some(String::new());
            log::info!("Command mode activated (type command, Enter to run, Esc to cancel)");
        }
        Action::ToggleFilter => {
            state.ui_state.filter.open = !state.ui_state.filter.open;
            log::info!("Filter mode: {}", state.ui_state.filter.open);
            state.scheduler.mark_dirty();
        }
        Action::ToggleSettings => {
            state.settings_state.open = !state.settings_state.open;
            log::info!("Settings toggled: {}", state.settings_state.open);
            state.scheduler.mark_dirty();
        }
    }
}
