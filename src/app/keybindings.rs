//! Sub-module extracted from app/mod.rs

use alacritty_terminal::grid::Dimensions;

use super::*;
use crate::ui::Ui;
use crate::ui::model::AppCtx;

// Re-export from ui layer.
pub(super) use crate::ui::keybindings::match_keybinding;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Copy the entire visible terminal grid into a String.
fn copy_full_screen(
    grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>,
    cols: usize,
    rows: usize,
) -> String {
    let mut buf = String::with_capacity(cols * rows + rows);
    for row_idx in 0..rows {
        let line = alacritty_terminal::index::Line(row_idx as i32);
        for col_idx in 0..cols {
            let col = alacritty_terminal::index::Column(col_idx);
            let c = grid[line][col].c;
            if c.is_control() || c == '\0' {
                buf.push(' ');
            } else {
                buf.push(c);
            }
        }
        let trimmed = buf.trim_end_matches(' ').len();
        buf.truncate(trimmed);
        if row_idx < rows - 1 {
            buf.push('\n');
        }
    }
    buf
}

// ─── Action Handler ──────────────────────────────────────────────────────────
impl Ui {
    pub fn handle_action(&mut self, ctx: &mut AppCtx<'_>, action: Action) {
        match action {
            Action::NewChat => {
                let id = *ctx.next_id;
                *ctx.next_id += 1;
                let shell = ctx
                    .config
                    .terminal
                    .shell
                    .clone()
                    .unwrap_or_else(crate::renderer::platform::default_shell);
                let phys = self.frame.window.inner_size();
                let logical = phys.to_logical::<f32>(self.frame.window.scale_factor());
                let (_, cols, rows) = ui::compute_single_pane(
                    logical.width as u32,
                    logical.height as u32,
                    &ctx.cell_size,
                    &self.styles,
                );
                let sandbox = ctx.active_sandbox_policy();
                let session = TerminalSession::new(
                    id,
                    &shell,
                    cols,
                    rows,
                    ctx.config.terminal.scrollback_lines,
                    Some(&sandbox),
                    Some(ctx.proxy.clone()),
                );
                ctx.sessions.insert(id, session);
                tiles::add_chat_tab(&mut self.tile_tree, id, "cronymax");

                // Create per-session input line.
                let mut prompt_editor = PromptState::new();
                prompt_editor.visible = true;
                // Copy model list from an existing editor so the ComboBox is populated.
                if let Some(existing) = ctx.ui_state.prompt_editors.values().next() {
                    prompt_editor.model_items = existing.model_items.clone();
                    prompt_editor.selected_model_idx = existing.selected_model_idx;
                }
                ctx.ui_state.prompt_editors.insert(id, prompt_editor);

                // Create per-session chat state.
                let (max_ctx, res) = llm_context_limits_split(ctx);
                let mut chat = crate::ui::chat::SessionChat::new(max_ctx, res);
                if let Some(sp) = ctx
                    .llm_client
                    .as_ref()
                    .and_then(|c| c.system_prompt().map(String::from))
                {
                    let model = llm_model_name_split(ctx);
                    chat.set_system_prompt(&sp, ctx.token_counter, &model);
                }
                ctx.session_chats.insert(id, chat);

                // Add to ui_state tabs.
                ctx.ui_state.tabs.push(TabInfo::Chat {
                    session_id: id,
                    title: "cronymax".into(),
                });
                ctx.ui_state.active_tab = ctx.ui_state.tabs.len() - 1;

                log::info!("New tab: session {}", id);
                ctx.scheduler.mark_dirty();
            }
            Action::NewTerminal => {
                let id = *ctx.next_id;
                *ctx.next_id += 1;
                let shell = ctx
                    .config
                    .terminal
                    .shell
                    .clone()
                    .unwrap_or_else(crate::renderer::platform::default_shell);
                let phys = self.frame.window.inner_size();
                let logical = phys.to_logical::<f32>(self.frame.window.scale_factor());
                let (_, cols, rows) = ui::compute_single_pane(
                    logical.width as u32,
                    logical.height as u32,
                    &ctx.cell_size,
                    &self.styles,
                );
                let sandbox = ctx.active_sandbox_policy();
                let session = TerminalSession::new(
                    id,
                    &shell,
                    cols,
                    rows,
                    ctx.config.terminal.scrollback_lines,
                    Some(&sandbox),
                    Some(ctx.proxy.clone()),
                );
                ctx.sessions.insert(id, session);
                tiles::add_terminal_tab(&mut self.tile_tree, id, "cronymax");

                // Terminal mode: prompt editor hidden.
                let mut prompt_editor = PromptState::new();
                prompt_editor.visible = false;
                ctx.ui_state.prompt_editors.insert(id, prompt_editor);

                // Create per-session chat state (still needed for AI interaction).
                let (max_ctx, res) = llm_context_limits_split(ctx);
                let mut chat = crate::ui::chat::SessionChat::new(max_ctx, res);
                if let Some(sp) = ctx
                    .llm_client
                    .as_ref()
                    .and_then(|c| c.system_prompt().map(String::from))
                {
                    let model = llm_model_name_split(ctx);
                    chat.set_system_prompt(&sp, ctx.token_counter, &model);
                }
                ctx.session_chats.insert(id, chat);

                // Add to ui_state tabs as terminal.
                ctx.ui_state.tabs.push(TabInfo::Terminal {
                    session_id: id,
                    title: "cronymax".into(),
                });
                ctx.ui_state.active_tab = ctx.ui_state.tabs.len() - 1;

                log::info!("New terminal tab: session {}", id);
                ctx.scheduler.mark_dirty();
            }
            Action::CloseTab => {
                if let Some(sid) = tiles::active_terminal_session(&self.tile_tree) {
                    ctx.sessions.remove(&sid);
                    tiles::remove_terminal_pane(&mut self.tile_tree, sid);
                    ctx.ui_state.prompt_editors.remove(&sid);
                    // Clean up per-session chat (abort any active stream).
                    if let Some(mut chat) = ctx.session_chats.remove(&sid)
                        && let Some(handle) = chat.active_stream.take()
                    {
                        handle.abort();
                    }
                    ctx.ui_state.tabs.retain(
                    |t| !matches!(t, TabInfo::Chat { session_id: s, .. } | TabInfo::Terminal { session_id: s, .. } if *s == sid),
                );
                    if ctx.ui_state.active_tab >= ctx.ui_state.tabs.len() {
                        ctx.ui_state.active_tab = ctx.ui_state.tabs.len().saturating_sub(1);
                    }
                    log::info!("Closed tab: session {}", sid);
                    ctx.scheduler.mark_dirty();
                }
            }
            Action::NextTab => {
                tiles::next_terminal_tab(&mut self.tile_tree);
                ctx.scheduler.mark_dirty();
            }
            Action::PrevTab => {
                tiles::prev_terminal_tab(&mut self.tile_tree);
                ctx.scheduler.mark_dirty();
            }
            Action::Copy => {
                if let Some(sid) = tiles::active_terminal_session(&self.tile_tree)
                    && let Some(session) = ctx.sessions.get(&sid)
                {
                    let term = session.state.term();
                    let grid = term.grid();
                    let cols = (session.grid_size.cols as usize).min(term.columns());
                    let rows = (session.grid_size.rows as usize).min(term.screen_lines());

                    let text_buf = if let Some(sel) = &self.terminal_selection {
                        if sel.session_id == sid {
                            // Copy only the selected region.
                            let (sc, sr, ec, er) = sel.normalized();
                            let mut buf = String::new();
                            for row in sr..=er {
                                if row >= rows {
                                    break;
                                }
                                let line = alacritty_terminal::index::Line(row as i32);
                                let c_start = if row == sr { sc } else { 0 };
                                let c_end = if row == er {
                                    ec.min(cols.saturating_sub(1))
                                } else {
                                    cols.saturating_sub(1)
                                };
                                for col_idx in c_start..=c_end {
                                    let col = alacritty_terminal::index::Column(col_idx);
                                    let c = grid[line][col].c;
                                    if c.is_control() || c == '\0' {
                                        buf.push(' ');
                                    } else {
                                        buf.push(c);
                                    }
                                }
                                let trimmed = buf.trim_end_matches(' ').len();
                                buf.truncate(trimmed);
                                if row < er {
                                    buf.push('\n');
                                }
                            }
                            buf
                        } else {
                            // Selection is for a different session; copy full screen.
                            copy_full_screen(grid, cols, rows)
                        }
                    } else {
                        copy_full_screen(grid, cols, rows)
                    };

                    let trimmed = text_buf.trim_end_matches('\n');
                    input::copy_to_clipboard(trimmed);
                    log::info!("Copied {} chars to clipboard", trimmed.len());
                    // Clear selection after copy.
                    self.terminal_selection = None;
                }
            }
            Action::Paste => {
                if let Some(text) = input::paste_from_clipboard()
                    && let Some(sid) = tiles::active_terminal_session(&self.tile_tree)
                    && let Some(session) = ctx.sessions.get_mut(&sid)
                {
                    session.write_to_pty(text.as_bytes());
                }
            }
            Action::FontSizeUp => {
                ctx.config.font.size = (ctx.config.font.size + 1.0).min(128.0);
                log::info!("Font size: {}", ctx.config.font.size);
                ctx.scheduler.mark_dirty();
            }
            Action::FontSizeDown => {
                ctx.config.font.size = (ctx.config.font.size - 1.0).max(1.0);
                log::info!("Font size: {}", ctx.config.font.size);
                ctx.scheduler.mark_dirty();
            }
            Action::ScrollUp => {
                if let Some(sid) = tiles::active_terminal_session(&self.tile_tree)
                    && let Some(session) = ctx.sessions.get_mut(&sid)
                {
                    session.state.scroll_up(3);
                    ctx.scheduler.mark_dirty();
                }
            }
            Action::ScrollDown => {
                if let Some(sid) = tiles::active_terminal_session(&self.tile_tree)
                    && let Some(session) = ctx.sessions.get_mut(&sid)
                {
                    session.state.scroll_down(3);
                    ctx.scheduler.mark_dirty();
                }
            }
            Action::ScrollPageUp => {
                if let Some(sid) = tiles::active_terminal_session(&self.tile_tree)
                    && let Some(session) = ctx.sessions.get_mut(&sid)
                {
                    session.state.scroll_page_up();
                    ctx.scheduler.mark_dirty();
                }
            }
            Action::ScrollPageDown => {
                if let Some(sid) = tiles::active_terminal_session(&self.tile_tree)
                    && let Some(session) = ctx.sessions.get_mut(&sid)
                {
                    session.state.scroll_page_down();
                    ctx.scheduler.mark_dirty();
                }
            }
            Action::SplitVertical | Action::SplitHorizontal => {
                let dir = if matches!(action, Action::SplitHorizontal) {
                    tiles::SplitDir::Horizontal
                } else {
                    tiles::SplitDir::Vertical
                };

                // Find the active tile to split.
                if let Some(active_sid) = tiles::active_terminal_session(&self.tile_tree)
                    && let Some(active_tile) = tiles::find_terminal_tile(&self.tile_tree, active_sid)
                {
                    // Create a new terminal session for the new pane.
                    let id = *ctx.next_id;
                    *ctx.next_id += 1;
                    let shell = ctx
                        .config
                        .terminal
                        .shell
                        .clone()
                        .unwrap_or_else(crate::renderer::platform::default_shell);
                    let (_, cols, rows) = ui::compute_single_pane(
                        self.frame.window.inner_size().width,
                        self.frame.window.inner_size().height,
                        &ctx.cell_size,
                        &self.styles,
                    );
                    let sandbox = ctx.active_sandbox_policy();
                    let session = TerminalSession::new(
                        id,
                        &shell,
                        cols,
                        rows,
                        ctx.config.terminal.scrollback_lines,
                        Some(&sandbox),
                        Some(ctx.proxy.clone()),
                    );
                    ctx.sessions.insert(id, session);

                    // Inherit the mode from the active tab.
                    let is_active_terminal = ctx
                        .ui_state
                        .prompt_editors
                        .get(&active_sid)
                        .is_some_and(|pe| !pe.visible);
                    let mut prompt_editor = PromptState::new();
                    prompt_editor.visible = !is_active_terminal;
                    ctx.ui_state.prompt_editors.insert(id, prompt_editor);

                    ctx.ui_state.tabs.push(if is_active_terminal {
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
                    tiles::split_pane_dir(&mut self.tile_tree, active_tile, new_pane, dir, true);
                    log::info!("Split {:?}: new session {}", dir, id);
                    ctx.scheduler.mark_dirty();
                }
            }
            Action::CommandMode => {
                ctx.ui_state.command_palette.open();
                log::info!("Command palette opened");
                ctx.scheduler.mark_dirty();
            }
            Action::ToggleFilter => {
                ctx.ui_state.filter.toggle();
                log::info!("Filter mode: {}", ctx.ui_state.filter.open);
                ctx.scheduler.mark_dirty();
            }
            Action::ToggleSettings => {
                ctx.ui_state.settings_state.open = !ctx.ui_state.settings_state.open;
                log::info!("Settings toggled: {}", ctx.ui_state.settings_state.open);
                ctx.scheduler.mark_dirty();
            }
        }
    }

    /// Create a new terminal tab with a specific shell program.
    pub fn new_terminal_with_shell(&mut self, ctx: &mut AppCtx<'_>, shell: &str) {
        let id = *ctx.next_id;
        *ctx.next_id += 1;
        let phys = self.frame.window.inner_size();
        let logical = phys.to_logical::<f32>(self.frame.window.scale_factor());
        let (_, cols, rows) = ui::compute_single_pane(
            logical.width as u32,
            logical.height as u32,
            &ctx.cell_size,
            &self.styles,
        );
        let sandbox = ctx.active_sandbox_policy();
        let session = TerminalSession::new(
            id,
            shell,
            cols,
            rows,
            ctx.config.terminal.scrollback_lines,
            Some(&sandbox),
            Some(ctx.proxy.clone()),
        );
        ctx.sessions.insert(id, session);

        let shell_name = std::path::Path::new(shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(shell);
        tiles::add_terminal_tab(&mut self.tile_tree, id, shell_name);

        let mut prompt_editor = PromptState::new();
        prompt_editor.visible = false;
        ctx.ui_state.prompt_editors.insert(id, prompt_editor);

        let (max_ctx, res) = llm_context_limits_split(ctx);
        let mut chat = crate::ui::chat::SessionChat::new(max_ctx, res);
        if let Some(sp) = ctx
            .llm_client
            .as_ref()
            .and_then(|c| c.system_prompt().map(String::from))
        {
            let model = llm_model_name_split(ctx);
            chat.set_system_prompt(&sp, ctx.token_counter, &model);
        }
        ctx.session_chats.insert(id, chat);

        ctx.ui_state.tabs.push(TabInfo::Terminal {
            session_id: id,
            title: shell_name.to_string(),
        });
        ctx.ui_state.active_tab = ctx.ui_state.tabs.len() - 1;

        log::info!("New terminal tab with shell '{}': session {}", shell, id);
        ctx.scheduler.mark_dirty();
    }

    /// Open a history tab showing past sessions (with message previews) and scheduled tasks.
    pub fn open_history_tab(&mut self, ctx: &mut AppCtx<'_>) {
        // Reuse existing history chat tab if already open.
        for (i, tab) in ctx.ui_state.tabs.iter().enumerate() {
            if let TabInfo::Chat { title, session_id } = tab
                && title == "History"
            {
                tiles::activate_terminal_tab(&mut self.tile_tree, *session_id);
                ctx.ui_state.active_tab = i;
                ctx.scheduler.mark_dirty();
                return;
            }
        }

        // Create a new chat tab that displays history content.
        let id = *ctx.next_id;
        *ctx.next_id += 1;
        let shell = ctx
            .config
            .terminal
            .shell
            .clone()
            .unwrap_or_else(crate::renderer::platform::default_shell);
        let phys = self.frame.window.inner_size();
        let logical = phys.to_logical::<f32>(self.frame.window.scale_factor());
        let (_, cols, rows) = ui::compute_single_pane(
            logical.width as u32,
            logical.height as u32,
            &ctx.cell_size,
            &self.styles,
        );
        let sandbox = ctx.active_sandbox_policy();
        let session = TerminalSession::new(
            id,
            &shell,
            cols,
            rows,
            ctx.config.terminal.scrollback_lines,
            Some(&sandbox),
            Some(ctx.proxy.clone()),
        );
        ctx.sessions.insert(id, session);
        tiles::add_chat_tab(&mut self.tile_tree, id, "History");

        let mut prompt_editor = PromptState::new();
        prompt_editor.visible = true;
        if let Some(existing) = ctx.ui_state.prompt_editors.values().next() {
            prompt_editor.model_items = existing.model_items.clone();
            prompt_editor.selected_model_idx = existing.selected_model_idx;
        }
        ctx.ui_state.prompt_editors.insert(id, prompt_editor);

        let (max_ctx, res) = llm_context_limits_split(ctx);
        let mut chat = crate::ui::chat::SessionChat::new(max_ctx, res);
        if let Some(sp) = ctx
            .llm_client
            .as_ref()
            .and_then(|c| c.system_prompt().map(String::from))
        {
            let model = llm_model_name_split(ctx);
            chat.set_system_prompt(&sp, ctx.token_counter, &model);
        }
        ctx.session_chats.insert(id, chat);

        // Build history content from saved sessions.
        let mut history_lines = Vec::new();
        history_lines.push("# Session History\n\n".to_string());

        if let Ok(mgr) = ctx.profile_manager.lock() {
            let profile_dir = mgr
                .active()
                .map(|p| mgr.profile_dir(&p.id))
                .unwrap_or_else(|| mgr.profile_dir("default"));
            let sessions_dir = profile_dir.join("sessions");
            if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
                let mut session_records: Vec<(
                    String,
                    crate::app::session_persist::ChatSessionRecord,
                )> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                    .filter_map(|e| {
                        let uuid = e.path().file_stem()?.to_str()?.to_string();
                        crate::app::session_persist::load_session_file(&uuid, &profile_dir)
                            .ok()
                            .map(|r| (uuid, r))
                    })
                    .collect();
                session_records.sort_by_key(|b| std::cmp::Reverse(b.1.updated_at));

                if session_records.is_empty() {
                    history_lines.push("*No saved sessions found.*\n".to_string());
                } else {
                    for (uuid, record) in session_records.iter().take(50) {
                        let msg_count = record.messages.len();
                        let model = if record.model.is_empty() {
                            "unknown"
                        } else {
                            &record.model
                        };
                        let ts = chrono_format_epoch(record.updated_at);

                        // Session header with resume link.
                        history_lines.push(format!(
                        "### [Resume ↗](cronymax://resume-session/{}) — {} messages, `{}`, {}\n\n",
                        uuid, msg_count, model, ts
                    ));

                        // Message preview: show first user message and last assistant response.
                        let first_user = record
                            .messages
                            .iter()
                            .find(|m| m.role == crate::ai::context::MessageRole::User);
                        let last_assistant = record
                            .messages
                            .iter()
                            .rev()
                            .find(|m| m.role == crate::ai::context::MessageRole::Assistant);

                        if let Some(msg) = first_user {
                            let preview = truncate_preview(&msg.content, 120);
                            history_lines.push(format!("**User:** {}\n\n", preview));
                        }
                        if let Some(msg) = last_assistant {
                            let preview = truncate_preview(&msg.content, 200);
                            history_lines.push(format!("**Assistant:** {}\n\n", preview));
                        }
                        history_lines.push("---\n\n".to_string());
                    }
                }
            } else {
                history_lines.push("*No sessions directory found.*\n".to_string());
            }
        }

        let history_content = history_lines.join("");

        // Store history as pinned content on the chat session (single pinned block).
        if let Some(chat) = ctx.session_chats.get_mut(&id) {
            chat.pinned_content = Some(history_content);
        }

        ctx.ui_state.tabs.push(TabInfo::Chat {
            session_id: id,
            title: "History".into(),
        });
        ctx.ui_state.active_tab = ctx.ui_state.tabs.len() - 1;

        log::info!("Opened history tab: session {}", id);
        ctx.scheduler.mark_dirty();
    }

    /// Open a dedicated Scheduled Tasks tab showing configured tasks and recent execution logs.
    pub fn open_scheduler_tab(&mut self, ctx: &mut AppCtx<'_>) {
        // Reuse existing scheduler tab if already open.
        for (i, tab) in ctx.ui_state.tabs.iter().enumerate() {
            if let TabInfo::Chat { title, session_id } = tab
                && title == "Schedule"
            {
                tiles::activate_terminal_tab(&mut self.tile_tree, *session_id);
                ctx.ui_state.active_tab = i;
                ctx.scheduler.mark_dirty();
                return;
            }
        }

        // Create a new chat tab for the scheduler view.
        let id = *ctx.next_id;
        *ctx.next_id += 1;
        let shell = ctx
            .config
            .terminal
            .shell
            .clone()
            .unwrap_or_else(crate::renderer::platform::default_shell);
        let phys = self.frame.window.inner_size();
        let logical = phys.to_logical::<f32>(self.frame.window.scale_factor());
        let (_, cols, rows) = ui::compute_single_pane(
            logical.width as u32,
            logical.height as u32,
            &ctx.cell_size,
            &self.styles,
        );
        let sandbox = ctx.active_sandbox_policy();
        let session = TerminalSession::new(
            id,
            &shell,
            cols,
            rows,
            ctx.config.terminal.scrollback_lines,
            Some(&sandbox),
            Some(ctx.proxy.clone()),
        );
        ctx.sessions.insert(id, session);
        tiles::add_chat_tab(&mut self.tile_tree, id, "Schedule");

        let mut prompt_editor = PromptState::new();
        prompt_editor.visible = true;
        if let Some(existing) = ctx.ui_state.prompt_editors.values().next() {
            prompt_editor.model_items = existing.model_items.clone();
            prompt_editor.selected_model_idx = existing.selected_model_idx;
        }
        ctx.ui_state.prompt_editors.insert(id, prompt_editor);

        let (max_ctx, res) = llm_context_limits_split(ctx);
        let mut chat = crate::ui::chat::SessionChat::new(max_ctx, res);
        if let Some(sp) = ctx
            .llm_client
            .as_ref()
            .and_then(|c| c.system_prompt().map(String::from))
        {
            let model = llm_model_name_split(ctx);
            chat.set_system_prompt(&sp, ctx.token_counter, &model);
        }

        // Build scheduler content.
        let mut lines = Vec::new();
        lines.push("# Scheduled Tasks\n\n".to_string());

        if ctx.task_store.tasks.is_empty() {
            lines.push("*No scheduled tasks configured.*\n\n".to_string());
            lines.push("Use **Settings → Scheduled Tasks** to create tasks.\n".to_string());
        } else {
            for task in &ctx.task_store.tasks {
                let status = if task.enabled { "✅" } else { "⏸" };
                lines.push(format!(
                    "### {} {}\n\n- **Cron:** `{}`\n- **Type:** {}\n- **Value:** `{}`\n\n",
                    status, task.name, task.cron, task.action_type, task.action_value
                ));
            }
        }

        lines.push("---\n\n## Recent Executions\n\n".to_string());
        if ctx.scheduler_history_cache.is_empty() {
            lines.push("*No execution records yet.*\n".to_string());
        } else {
            for record in ctx.scheduler_history_cache.iter().take(50) {
                let status_icon = if record.status == "success" {
                    "✅"
                } else {
                    "❌"
                };
                let output_preview = if record.output.len() > 120 {
                    format!("{}…", &record.output[..120])
                } else {
                    record.output.clone()
                };
                lines.push(format!(
                    "- {} task `{}` at {} ({}ms): {}\n",
                    status_icon,
                    record.task_id,
                    record.timestamp,
                    record.duration_ms,
                    output_preview
                ));
            }
        }

        chat.pinned_content = Some(lines.join(""));
        ctx.session_chats.insert(id, chat);

        ctx.ui_state.tabs.push(TabInfo::Chat {
            session_id: id,
            title: "Schedule".into(),
        });
        ctx.ui_state.active_tab = ctx.ui_state.tabs.len() - 1;

        log::info!("Opened scheduler tab: session {}", id);
        ctx.scheduler.mark_dirty();
    }

    /// Open a saved session in a new chat tab, restoring its message history.
    pub fn open_history_session(&mut self, ctx: &mut AppCtx<'_>, uuid: &str) {
        // Load session record from disk.
        let record = {
            let mgr = match ctx.profile_manager.lock() {
                Ok(m) => m,
                Err(_) => {
                    log::error!("Failed to lock profile manager");
                    return;
                }
            };
            let profile_dir = mgr
                .active()
                .map(|p| mgr.profile_dir(&p.id))
                .unwrap_or_else(|| mgr.profile_dir("default"));
            match crate::app::session_persist::load_session_file(uuid, &profile_dir) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Failed to load session {}: {}", uuid, e);
                    return;
                }
            }
        };

        // Check if this session is already open.
        for (i, tab) in ctx.ui_state.tabs.iter().enumerate() {
            if let TabInfo::Chat { session_id, .. } = tab
                && let Some(chat) = ctx.session_chats.get(session_id)
                && chat.persistent_id.as_deref() == Some(uuid)
            {
                tiles::activate_terminal_tab(&mut self.tile_tree, *session_id);
                ctx.ui_state.active_tab = i;
                ctx.scheduler.mark_dirty();
                return;
            }
        }

        // Create session infrastructure.
        let id = *ctx.next_id;
        *ctx.next_id += 1;
        let shell = ctx
            .config
            .terminal
            .shell
            .clone()
            .unwrap_or_else(crate::renderer::platform::default_shell);
        let phys = self.frame.window.inner_size();
        let logical = phys.to_logical::<f32>(self.frame.window.scale_factor());
        let (_, cols, rows) = ui::compute_single_pane(
            logical.width as u32,
            logical.height as u32,
            &ctx.cell_size,
            &self.styles,
        );
        let sandbox = ctx.active_sandbox_policy();
        let session = TerminalSession::new(
            id,
            &shell,
            cols,
            rows,
            ctx.config.terminal.scrollback_lines,
            Some(&sandbox),
            Some(ctx.proxy.clone()),
        );
        ctx.sessions.insert(id, session);

        // Derive a short title from the first user message.
        let title = record
            .messages
            .iter()
            .find(|m| m.role == crate::ai::context::MessageRole::User)
            .map(|m| {
                let t = m.content.chars().take(30).collect::<String>();
                if m.content.chars().count() > 30 {
                    format!("{}…", t)
                } else {
                    t
                }
            })
            .unwrap_or_else(|| "Resumed".to_string());

        tiles::add_chat_tab(&mut self.tile_tree, id, &title);

        let mut prompt_editor = PromptState::new();
        prompt_editor.visible = true;
        if let Some(existing) = ctx.ui_state.prompt_editors.values().next() {
            prompt_editor.model_items = existing.model_items.clone();
            prompt_editor.selected_model_idx = existing.selected_model_idx;
        }
        ctx.ui_state.prompt_editors.insert(id, prompt_editor);

        let (max_ctx, res) = llm_context_limits_split(ctx);
        let mut chat = crate::ui::chat::SessionChat::new(max_ctx, res);
        chat.persistent_id = Some(uuid.to_string());

        // Set system prompt if available.
        if let Some(sp) = ctx
            .llm_client
            .as_ref()
            .and_then(|c| c.system_prompt().map(String::from))
        {
            let model = llm_model_name_split(ctx);
            chat.set_system_prompt(&sp, ctx.token_counter, &model);
        }

        // Restore messages from the saved session.
        let model_name = llm_model_name_split(ctx);
        for msg in &record.messages {
            // Push into history (for LLM context).
            let tc = ctx.token_counter.count(&msg.content, &model_name) as u32;
            let mut restored = msg.clone();
            restored.token_count = tc;
            chat.history.push(restored.clone());
            // Add to display messages.
            chat.add_message(restored);
        }

        // Also display restored messages as blocks in the prompt editor.
        let pe = ctx.ui_state.prompt_editors.get_mut(&id).unwrap();
        let mut block_id = 0u32;
        let mut i = 0;
        while i < record.messages.len() {
            let msg = &record.messages[i];
            if msg.role == crate::ai::context::MessageRole::User {
                // Look for the next assistant response.
                let response = record
                    .messages
                    .get(i + 1)
                    .filter(|m| m.role == crate::ai::context::MessageRole::Assistant)
                    .map(|m| m.content.clone())
                    .unwrap_or_default();
                pe.blocks.push(Block::Stream {
                    id: block_id,
                    prompt: msg.content.clone(),
                    response,
                    is_streaming: false,
                    tool_status: None,
                    tool_calls_log: vec![],
                });
                block_id += 1;
                // Skip the assistant message if we consumed it.
                if i + 1 < record.messages.len()
                    && record.messages[i + 1].role == crate::ai::context::MessageRole::Assistant
                {
                    i += 2;
                } else {
                    i += 1;
                }
            } else if msg.role == crate::ai::context::MessageRole::Assistant {
                // Standalone assistant message (no preceding user msg).
                pe.blocks.push(Block::Stream {
                    id: block_id,
                    prompt: String::new(),
                    response: msg.content.clone(),
                    is_streaming: false,
                    tool_status: None,
                    tool_calls_log: vec![],
                });
                block_id += 1;
                i += 1;
            } else {
                i += 1;
            }
        }

        ctx.session_chats.insert(id, chat);

        ctx.ui_state.tabs.push(TabInfo::Chat {
            session_id: id,
            title: title.clone(),
        });
        ctx.ui_state.active_tab = ctx.ui_state.tabs.len() - 1;

        log::info!(
            "Opened history session '{}' as tab: session {} ({})",
            uuid,
            id,
            title
        );
        ctx.scheduler.mark_dirty();
    }
}

/// Format epoch millis as a human-readable date string.
fn chrono_format_epoch(epoch_ms: u64) -> String {
    let secs = (epoch_ms / 1000) as i64;
    let dt = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
    let datetime: std::time::SystemTime = dt;
    // Simple formatting without chrono dependency.
    format!("{:?}", datetime)
}

/// Truncate a string to `max_chars`, appending "…" if truncated.
fn truncate_preview(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    // Collapse to single line for preview.
    let line: String = trimmed
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if line.chars().count() > max_chars {
        let t: String = line.chars().take(max_chars).collect();
        format!("{}…", t)
    } else {
        line
    }
}
