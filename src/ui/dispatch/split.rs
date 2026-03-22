//! Terminal split-pane commands extracted from cmd_webview.rs

use crate::renderer::scheduler::RenderSchedule;
use crate::renderer::terminal::TerminalSession;
use crate::ui::{self, Ui, UiAction, model::AppCtx, prompt::PromptState, tiles};

impl Ui {
    pub(crate) fn handle_split(&mut self, ctx: &mut AppCtx<'_>, action: &UiAction) {
        let (dir, insert_after) = match action {
            UiAction::SplitLeft => (tiles::SplitDir::Horizontal, false),
            UiAction::SplitRight => (tiles::SplitDir::Horizontal, true),
            _ => (tiles::SplitDir::Vertical, true), // SplitDown
        };
        if let Some(active_sid) = tiles::active_terminal_session(&self.tile_tree)
            && let Some(active_tile) = tiles::find_terminal_tile(&self.tile_tree, active_sid)
        {
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
            // Inherit the mode from the active tab so Terminal→Terminal
            // and Chat→Chat.
            let is_active_terminal = ctx.ui_state
                .prompt_editors
                .get(&active_sid)
                .is_some_and(|pe| !pe.visible);
            let mut prompt_editor = PromptState::new();
            prompt_editor.visible = !is_active_terminal;
            ctx.ui_state.prompt_editors.insert(id, prompt_editor);
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
                &mut self.tile_tree,
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
            ctx.scheduler.mark_dirty();
        }
    }
}
