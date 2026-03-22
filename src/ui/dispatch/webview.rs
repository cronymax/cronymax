//! Webview/tab/dock UI action handlers extracted from commands.rs

use crate::renderer::scheduler::RenderSchedule;

use crate::renderer::viewport::Viewport;
use crate::ui::{
    BrowserViewMode, Ui, UiAction, ViewMut, actions::KeyAction as Action, model::AppCtx, tiles,
    types::TabInfo,
};

impl Ui {
    pub(crate) fn handle_ui_action_webview(
        &mut self,
        ctx: &mut AppCtx<'_>,
        action: UiAction,
        #[allow(unused)] event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        match action {
            UiAction::SwitchTab(idx) => {
                if let Some(
                    TabInfo::Chat { session_id, .. } | TabInfo::Terminal { session_id, .. },
                ) = ctx.ui_state.tabs.get(idx)
                {
                    let sid = *session_id;

                    // Reset focused session — the new tab's first pane will
                    // become focused on the next click.
                    ctx.ui_state.focused_terminal_session = Some(sid);

                    // Pane is always in the tile tree (even when pinned, just hidden from tab bar).
                    tiles::activate_terminal_tab(&mut self.tile_tree, sid);

                    // For overlay webviews: show paired ones, hide others.
                    // For docked webviews: leave visibility alone — the tile
                    // positioning code in RedrawRequested manages them.
                    let mut restored_webview: Option<usize> = None;
                    for (i, wt) in self.browser_tabs.iter_mut().enumerate() {
                        if wt.mode == BrowserViewMode::Overlay {
                            let is_paired = wt.paired_session == Some(sid);
                            wt.browser.view.set_visible(is_paired);
                            if is_paired && restored_webview.is_none() {
                                restored_webview = Some(i);
                            }
                        }
                    }

                    // Restore the paired overlay as active webview, or deselect.
                    if let Some(wv_idx) = restored_webview {
                        self.active_browser = wv_idx;
                        ctx.ui_state.active_browser = Some(wv_idx);
                    } else {
                        ctx.ui_state.active_browser = None;
                    }

                    log::info!("Switched to terminal tab: session {}", sid);
                }
            }
            UiAction::CloseTab(sid) => {
                // sid is a session_id (u32). Clean up the session and input ctx.
                // Note: the tile is already removed by egui_tiles (on_tab_close returned true).
                ctx.sessions.remove(&sid);
                ctx.ui_state.prompt_editors.remove(&sid);
                // Clear focused session if it was the closed one.
                if ctx.ui_state.focused_terminal_session == Some(sid) {
                    ctx.ui_state.focused_terminal_session = None;
                }
                // Also remove from ui_state.tabs list.
                if let Some(idx) =
                ctx.ui_state.tabs.iter().position(
                    |t| matches!(t, TabInfo::Chat { session_id: s, .. } | TabInfo::Terminal { session_id: s, .. } if *s == sid),
                )
            {
                ctx.ui_state.tabs.remove(idx);
                if ctx.ui_state.active_tab >= ctx.ui_state.tabs.len() {
                    ctx.ui_state.active_tab = ctx.ui_state.tabs.len().saturating_sub(1);
                }
            }
                log::info!("Closed tab: session {}", sid);
            }
            UiAction::NewChat => crate::app::handle_action(self, ctx, Action::NewChat),
            UiAction::NewTerminal => crate::app::handle_action(self, ctx, Action::NewTerminal),
            UiAction::NewTerminalWithShell(shell) => {
                crate::app::new_terminal_with_shell(self, ctx, &shell);
            }
            UiAction::OpenHistory => {
                crate::app::open_history_tab(self, ctx);
            }
            UiAction::OpenHistorySession(uuid) => {
                crate::app::open_history_session(self, ctx, &uuid);
            }
            UiAction::OpenBrowserOverlay(url) => {
                crate::app::open_browser(self, ctx, &url, event_loop);
            }
            UiAction::ExecuteCommand(cmd) => self.handle_colon_command(ctx, &cmd, event_loop),
            UiAction::NavigateWebview(url, wid) => {
                // Look up the target webview by ID; fall back to active_webview.
                let idx = if wid != 0 {
                    self.browser_tabs.iter().position(|wt| wt.browser.id == wid)
                } else {
                    Some(self.active_browser)
                };
                if let Some(idx) = idx
                    && let Some(tab) = self.browser_tabs.get_mut(idx)
                {
                    let actual_wid = tab.browser.id;
                    tab.browser.navigate(&url);
                    // Keep the tile tree Pane URL in sync so the docked
                    // address bar reflects the navigation immediately.
                    tiles::update_browser_view_url(&mut self.tile_tree, actual_wid, &url);
                }
            }
            UiAction::SwitchWebview(idx) => crate::app::switch_browser_tab(self, ctx, idx),
            UiAction::ActivateWebviewPane(wid) => {
                tiles::activate_browser_view_tab(&mut self.tile_tree, wid);
                // Also set ui_state.active_webview to the matching index.
                if let Some(idx) = self.browser_tabs.iter().position(|wt| wt.browser.id == wid) {
                    self.active_browser = idx;
                    ctx.ui_state.active_browser = Some(idx);
                }
                log::info!("Activated webview pane {} in tile tree", wid);
            }
            UiAction::CloseWebview(wid) => {
                // wid is a webview_id (u32). Find and remove by ID.
                if let Some(idx) = self.browser_tabs.iter().position(|wt| wt.browser.id == wid) {
                    self.browser_tabs.remove(idx);
                    // Remove from z-stack to prevent stale overlay z-order entries.
                    self.browser_manager.unregister(wid);
                    // Also remove from tile tree if it was added there.
                    tiles::remove_browser_view_pane(&mut self.tile_tree, wid);
                    if self.browser_tabs.is_empty() {
                        ctx.ui_state.active_browser = None;
                        crate::app::close_active_browser(self, ctx);
                    } else if self.active_browser >= self.browser_tabs.len() {
                        self.active_browser = self.browser_tabs.len() - 1;
                        ctx.ui_state.active_browser = Some(self.active_browser);
                    }

                    // Hide float panel if no overlay webviews remain.
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    {
                        let has_overlays = self.browser_tabs.iter().any(|wt| {
                            wt.mode == BrowserViewMode::Overlay && wt.browser.view.visible
                        });
                        if !has_overlays {
                            self.float_panel_state.clear();
                            if let Some(ref mut fr) = self.float_renderer {
                                fr.set_visible(false);
                            }
                        }
                    }
                }
            }
            UiAction::WebviewBack(wid) => {
                let idx = if wid != 0 {
                    self.browser_tabs.iter().position(|wt| wt.browser.id == wid)
                } else {
                    Some(self.active_browser)
                };
                if let Some(idx) = idx
                    && let Some(tab) = self.browser_tabs.get(idx)
                {
                    tab.browser.go_back();
                }
            }
            UiAction::WebviewForward(wid) => {
                let idx = if wid != 0 {
                    self.browser_tabs.iter().position(|wt| wt.browser.id == wid)
                } else {
                    Some(self.active_browser)
                };
                if let Some(idx) = idx
                    && let Some(tab) = self.browser_tabs.get(idx)
                {
                    tab.browser.go_forward();
                }
            }
            UiAction::WebviewRefresh(wid) => {
                let idx = if wid != 0 {
                    self.browser_tabs.iter().position(|wt| wt.browser.id == wid)
                } else {
                    Some(self.active_browser)
                };
                if let Some(idx) = idx
                    && let Some(tab) = self.browser_tabs.get_mut(idx)
                {
                    tab.browser.refresh();
                }
            }
            UiAction::FilterSearch(query) => {
                // Search for query text in the focused terminal session.
                if let Some(sid) = ctx.ui_state.focused_terminal_session
                    && let Some(session) = ctx.sessions.get_mut(&sid)
                {
                    let matches = session.state.search_text(&query);
                    ctx.ui_state.filter.text = query;
                    ctx.ui_state.filter.match_count = matches.len();
                    if !matches.is_empty() {
                        ctx.ui_state.filter.current_match = 1;
                        // Scroll to the first match.
                        let (line, _col) = matches[0];
                        session.state.scroll_to_line(line);
                        session.is_dirty = true;
                    } else {
                        ctx.ui_state.filter.current_match = 0;
                    }
                }
            }
            UiAction::FilterClose => {
                ctx.ui_state.filter.open = false;
            }
            UiAction::FilterNext | UiAction::FilterPrev => {
                // Navigate between search matches in the focused terminal.
                if let Some(sid) = ctx.ui_state.focused_terminal_session
                    && let Some(session) = ctx.sessions.get_mut(&sid)
                {
                    let matches = session.state.search_text(&ctx.ui_state.filter.text);
                    if !matches.is_empty() {
                        let count = matches.len();
                        let current = ctx.ui_state.filter.current_match;
                        let next = match action {
                            UiAction::FilterNext => {
                                if current >= count {
                                    1
                                } else {
                                    current + 1
                                }
                            }
                            UiAction::FilterPrev => {
                                if current <= 1 {
                                    count
                                } else {
                                    current - 1
                                }
                            }
                            _ => unreachable!(),
                        };
                        ctx.ui_state.filter.current_match = next;
                        ctx.ui_state.filter.match_count = count;
                        let (line, _col) = matches[next - 1];
                        session.state.scroll_to_line(line);
                        session.is_dirty = true;
                    }
                }
            }
            UiAction::DockTab {
                source,
                target,
                direction,
            } => {
                // Remove the source pane from the tree and re-insert it as a split
                // next to the target pane.  Always use split_pane_dir so the root
                // Tabs container stays flat (one tab bar, no nested Tabs).
                if let Some(target_tile) = tiles::find_terminal_tile(&self.tile_tree, target) {
                    // Remove source from its current position.
                    tiles::remove_terminal_pane(&mut self.tile_tree, source);

                    // Map dock direction to Linear direction + insertion order.
                    let (linear_dir, insert_after) = match direction {
                        tiles::DockDirection::Left => (tiles::SplitDir::Horizontal, false),
                        tiles::DockDirection::Right => (tiles::SplitDir::Horizontal, true),
                        tiles::DockDirection::Top => (tiles::SplitDir::Vertical, false),
                        tiles::DockDirection::Bottom => (tiles::SplitDir::Vertical, true),
                    };

                    // Re-create the source pane.
                    let title = ctx
                        .ui_state
                        .tabs
                        .iter()
                        .find_map(|t| match t {
                            TabInfo::Chat {
                                session_id, title, ..
                            }
                            | TabInfo::Terminal {
                                session_id, title, ..
                            } if *session_id == source => Some(title.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "cronymax".into());
                    let new_pane = tiles::Pane::Chat {
                        session_id: source,
                        title,
                    };

                    tiles::split_pane_dir(
                        &mut self.tile_tree,
                        target_tile,
                        new_pane,
                        linear_dir,
                        insert_after,
                    );

                    tiles::activate_terminal_tab(&mut self.tile_tree, source);
                    // Force both sessions dirty so the new layout renders immediately.
                    for sid in [source, target] {
                        if let Some(s) = ctx.sessions.get_mut(&sid) {
                            s.is_dirty = true;
                        }
                    }
                    ctx.scheduler.mark_dirty();
                    log::info!(
                        "Docked session {} {:?} relative to session {}",
                        source,
                        direction,
                        target
                    );
                }
            }
            UiAction::DockWebview
            | UiAction::DockWebviewLeft
            | UiAction::DockWebviewRight
            | UiAction::DockWebviewDown => {
                // Dock the active overlay webview as a split in the egui_tiles tree.
                let (dir, insert_after) = match action {
                    UiAction::DockWebviewLeft => (tiles::SplitDir::Horizontal, false),
                    UiAction::DockWebviewDown => (tiles::SplitDir::Vertical, true),
                    _ => (tiles::SplitDir::Horizontal, true), // DockWebview / DockWebviewRight
                };
                if let Some(idx) = ctx.ui_state.active_browser
                    && let Some(tab) = self.browser_tabs.get_mut(idx)
                {
                    tab.mode = BrowserViewMode::Docked;

                    // If the webview was in an overlay panel, reparent it
                    // back to the main window and destroy the overlay renderer.
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    let had_overlay = tab.overlay.is_some();
                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                    let had_overlay = false;
                    if had_overlay {
                        tab.browser.view.reparent_to_window(&self.frame.window);
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        {
                            tab.overlay = None;
                        }
                    }

                    // Move the native webview to zero-size off-screen instead of
                    // hiding it, so that the next frame's positioning code can
                    // reposition it without a WKWebView show/hide latency flash.
                    let offscreen = Viewport::new(0.0, 0.0, 1.0, 1.0);
                    tab.browser.view.set_viewport(offscreen);
                    let wid = tab.browser.id;
                    let title = tab.browser.title.clone();
                    let url = tab.browser.url.clone();
                    self.active_browser = idx;

                    // Sync the z-order manager.
                    self.browser_manager.demote_to_docked(wid);

                    // Hide float panel if no overlay webviews remain after demotion.
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    {
                        let has_overlays = self.browser_tabs.iter().any(|wt| {
                            wt.mode == BrowserViewMode::Overlay && wt.browser.view.visible
                        });
                        if !has_overlays {
                            self.float_panel_state.clear();
                            if let Some(ref mut fr) = self.float_renderer {
                                fr.set_visible(false);
                            }
                        }
                    }

                    // Find the active terminal tile and split with the webview pane.
                    if let Some(active_sid) = tiles::active_terminal_session(&self.tile_tree)
                        && let Some(active_tile) =
                            tiles::find_terminal_tile(&self.tile_tree, active_sid)
                    {
                        let webview_pane = tiles::Pane::BrowserView {
                            webview_id: wid,
                            title,
                            url,
                        };
                        tiles::split_pane_dir(
                            &mut self.tile_tree,
                            active_tile,
                            webview_pane,
                            dir,
                            insert_after,
                        );
                        // Re-activate the terminal tab so the split doesn't
                        // switch focus to the newly-docked webview pane.
                        tiles::activate_terminal_tab(&mut self.tile_tree, active_sid);
                        log::info!("Docked webview {} as split {:?}", wid, dir);
                    } else {
                        // No active terminal pane — add as tab instead.
                        tiles::add_browser_view_tab(&mut self.tile_tree, wid, &title, &url);
                        log::info!("Docked webview {} as tab (no active terminal)", wid);
                    }
                    // Force egui repaint and window redraw so the next frame
                    // immediately picks up the new tile layout and repositions
                    // the native webview.
                    ctx.scheduler.mark_dirty();
                    ctx.scheduler.mark_dirty();
                } else {
                    log::warn!(
                        "DockWebview: no active webview (active_webview={:?})",
                        ctx.ui_state.active_browser
                    );
                }
            }
            UiAction::WebviewToTab(req_wid) => {
                // Resolve webview index: if req_wid != 0, look up by ID;
                // otherwise fall back to active_webview.
                let idx = if req_wid != 0 {
                    self.browser_tabs
                        .iter()
                        .position(|wt| wt.browser.id == req_wid)
                } else {
                    ctx.ui_state.active_browser
                };
                // Behaviour depends on the webview's current mode:
                // - Overlay → Docked tile pane  (existing logic)
                // - Docked  → Overlay tab       (move back to floating overlay)
                if let Some(idx) = idx
                    && let Some(tab) = self.browser_tabs.get_mut(idx)
                {
                    if tab.mode == BrowserViewMode::Docked {
                        // ── Docked → Overlay ──
                        // Remove from tile tree and switch to overlay mode.
                        let wid = tab.browser.id;
                        tiles::remove_browser_view_pane(&mut self.tile_tree, wid);
                        tab.mode = BrowserViewMode::Overlay;
                        tab.browser.view.visible = true;
                        self.browser_manager.promote_to_overlay(wid);
                        log::info!("Webview {} moved from split pane to overlay tab", wid);
                        ctx.scheduler.mark_dirty();
                    } else {
                        // ── Overlay → Docked tile ──
                        tab.mode = BrowserViewMode::Docked;

                        // If the webview was in an overlay panel, reparent it
                        // back to the main window and destroy the overlay renderer.
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        let had_overlay = tab.overlay.is_some();
                        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                        let had_overlay = false;
                        if had_overlay {
                            tab.browser.view.reparent_to_window(&self.frame.window);
                            #[cfg(any(target_os = "macos", target_os = "windows"))]
                            {
                                tab.overlay = None;
                            }
                        }

                        // Move off-screen instead of hiding to avoid WKWebView latency.
                        let offscreen = Viewport::new(0.0, 0.0, 1.0, 1.0);
                        tab.browser.view.set_viewport(offscreen);
                        let wid = tab.browser.id;
                        let title = tab.browser.title.clone();
                        let url = tab.browser.url.clone();
                        self.active_browser = idx;

                        // Sync the z-order manager.
                        self.browser_manager.demote_to_docked(wid);

                        // Hide float panel if no overlay webviews remain after tab conversion.
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        {
                            let has_overlays = self.browser_tabs.iter().any(|wt| {
                                wt.mode == BrowserViewMode::Overlay && wt.browser.view.visible
                            });
                            if !has_overlays {
                                self.float_panel_state.clear();
                                if let Some(ref mut fr) = self.float_renderer {
                                    fr.set_visible(false);
                                }
                            }
                        }

                        tiles::add_browser_view_tab(&mut self.tile_tree, wid, &title, &url);
                        log::info!("Webview {} moved to tile tab", wid);
                    }
                }
            }
            UiAction::OpenInSystemBrowser => {
                // Open the current webview URL in the system default browser.
                let url = if let Some(idx) = ctx.ui_state.active_browser
                    && let Some(tab) = self.browser_tabs.get(idx)
                {
                    Some(tab.browser.url.clone())
                } else {
                    // Try address bar URL as fallback.
                    let u = ctx.ui_state.address_bar.url.clone();
                    if u.is_empty() { None } else { Some(u) }
                };
                if let Some(url) = url {
                    log::info!("Opening in system browser: {}", url);
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                    }
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("cmd")
                            .args(["/C", "start", &url])
                            .spawn();
                    }
                }
            }
            UiAction::SplitLeft | UiAction::SplitRight | UiAction::SplitDown => {
                self.handle_split(ctx, &action);
            }
            _ => {}
        }
    }
}
