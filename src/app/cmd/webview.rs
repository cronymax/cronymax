//! Webview/tab/dock UI action handlers extracted from commands.rs

use crate::app::*;

pub(in crate::app) fn handle_ui_action_webview(
    state: &mut AppState,
    action: UiAction,
    #[allow(unused)] event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::SwitchTab(idx) => {
            if let Some(TabInfo::Chat { session_id, .. } | TabInfo::Terminal { session_id, .. }) =
                state.ui_state.tabs.get(idx)
            {
                let sid = *session_id;

                // Reset focused session — the new tab's first pane will
                // become focused on the next click.
                state.ui_state.focused_terminal_session = Some(sid);

                // Pane is always in the tile tree (even when pinned, just hidden from tab bar).
                tiles::activate_terminal_tab(&mut state.tile_tree, sid);

                // For overlay webviews: show paired ones, hide others.
                // For docked webviews: leave visibility alone — the tile
                // positioning code in RedrawRequested manages them.
                let mut restored_webview: Option<usize> = None;
                for (i, wt) in state.webview_tabs.iter_mut().enumerate() {
                    if wt.mode == BrowserViewMode::Overlay {
                        let is_paired = wt.paired_session == Some(sid);
                        wt.manager.set_visible(is_paired);
                        if is_paired && restored_webview.is_none() {
                            restored_webview = Some(i);
                        }
                    }
                }

                // Restore the paired overlay as active webview, or deselect.
                if let Some(wv_idx) = restored_webview {
                    state.active_webview = wv_idx;
                    state.ui_state.active_webview = Some(wv_idx);
                } else {
                    state.ui_state.active_webview = None;
                }

                log::info!("Switched to terminal tab: session {}", sid);
            }
        }
        UiAction::CloseTab(sid) => {
            // sid is a session_id (u32). Clean up the session and input state.
            // Note: the tile is already removed by egui_tiles (on_tab_close returned true).
            state.sessions.remove(&sid);
            state.prompt_editors.remove(&sid);
            // Clear focused session if it was the closed one.
            if state.ui_state.focused_terminal_session == Some(sid) {
                state.ui_state.focused_terminal_session = None;
            }
            // Also remove from ui_state.tabs list.
            if let Some(idx) =
                state.ui_state.tabs.iter().position(
                    |t| matches!(t, TabInfo::Chat { session_id: s, .. } | TabInfo::Terminal { session_id: s, .. } if *s == sid),
                )
            {
                state.ui_state.tabs.remove(idx);
                if state.ui_state.active_tab >= state.ui_state.tabs.len() {
                    state.ui_state.active_tab = state.ui_state.tabs.len().saturating_sub(1);
                }
            }
            log::info!("Closed tab: session {}", sid);
        }
        UiAction::NewChat => handle_action(state, Action::NewChat),
        UiAction::NewTerminal => handle_action(state, Action::NewTerminal),
        UiAction::ExecuteCommand(cmd) => handle_colon_command(state, &cmd, event_loop),
        UiAction::NavigateWebview(url, wid) => {
            // Look up the target webview by ID; fall back to active_webview.
            let idx = if wid != 0 {
                state.webview_tabs.iter().position(|wt| wt.id == wid)
            } else {
                Some(state.active_webview)
            };
            if let Some(idx) = idx
                && let Some(tab) = state.webview_tabs.get_mut(idx)
            {
                let actual_wid = tab.id;
                tab.manager.navigate(&url);
                tab.url = url.clone();
                // Keep the tile tree Pane URL in sync so the docked
                // address bar reflects the navigation immediately.
                tiles::update_browser_view_url(&mut state.tile_tree, actual_wid, &url);
            }
        }
        UiAction::SwitchWebview(idx) => switch_webview_tab(state, idx),
        UiAction::ActivateWebviewPane(wid) => {
            tiles::activate_browser_view_tab(&mut state.tile_tree, wid);
            // Also set ui_state.active_webview to the matching index.
            if let Some(idx) = state.webview_tabs.iter().position(|wt| wt.id == wid) {
                state.active_webview = idx;
                state.ui_state.active_webview = Some(idx);
            }
            log::info!("Activated webview pane {} in tile tree", wid);
        }
        UiAction::CloseWebview(wid) => {
            // wid is a webview_id (u32). Find and remove by ID.
            if let Some(idx) = state.webview_tabs.iter().position(|wt| wt.id == wid) {
                state.webview_tabs.remove(idx);
                // Remove from z-stack to prevent stale overlay z-order entries.
                state.webview_manager.unregister(wid);
                // Also remove from tile tree if it was added there.
                tiles::remove_browser_view_pane(&mut state.tile_tree, wid);
                if state.webview_tabs.is_empty() {
                    state.ui_state.active_webview = None;
                    close_active_webview(state);
                } else if state.active_webview >= state.webview_tabs.len() {
                    state.active_webview = state.webview_tabs.len() - 1;
                    state.ui_state.active_webview = Some(state.active_webview);
                }

                // Hide float panel if no overlay webviews remain.
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                {
                    let has_overlays = state
                        .webview_tabs
                        .iter()
                        .any(|wt| wt.mode == BrowserViewMode::Overlay && wt.manager.visible);
                    if !has_overlays {
                        state.float_panel_state.clear();
                        if let Some(ref mut fr) = state.float_renderer {
                            use crate::renderer::overlay::Renderer;
                            fr.set_visible(false);
                        }
                    }
                }
            }
        }
        UiAction::WebviewBack(wid) => {
            let idx = if wid != 0 {
                state.webview_tabs.iter().position(|wt| wt.id == wid)
            } else {
                Some(state.active_webview)
            };
            if let Some(idx) = idx
                && let Some(tab) = state.webview_tabs.get(idx)
            {
                let _ = tab.manager.webview.evaluate_script("window.history.back()");
            }
        }
        UiAction::WebviewForward(wid) => {
            let idx = if wid != 0 {
                state.webview_tabs.iter().position(|wt| wt.id == wid)
            } else {
                Some(state.active_webview)
            };
            if let Some(idx) = idx
                && let Some(tab) = state.webview_tabs.get(idx)
            {
                let _ = tab
                    .manager
                    .webview
                    .evaluate_script("window.history.forward()");
            }
        }
        UiAction::WebviewRefresh(wid) => {
            let idx = if wid != 0 {
                state.webview_tabs.iter().position(|wt| wt.id == wid)
            } else {
                Some(state.active_webview)
            };
            if let Some(idx) = idx
                && let Some(tab) = state.webview_tabs.get_mut(idx)
            {
                let url = tab.url.clone();
                tab.manager.navigate(&url);
            }
        }
        UiAction::FilterSearch(_query) => {
            // TODO: implement terminal search
        }
        UiAction::FilterClose => {
            state.ui_state.filter.open = false;
        }
        UiAction::FilterNext | UiAction::FilterPrev => {
            // TODO: implement search navigation
        }
        UiAction::DockTab {
            source,
            target,
            direction,
        } => {
            // Remove the source pane from the tree and re-insert it as a split
            // next to the target pane.  Always use split_pane_dir so the root
            // Tabs container stays flat (one tab bar, no nested Tabs).
            if let Some(target_tile) = tiles::find_terminal_tile(&state.tile_tree, target) {
                // Remove source from its current position.
                tiles::remove_terminal_pane(&mut state.tile_tree, source);

                // Map dock direction to Linear direction + insertion order.
                let (linear_dir, insert_after) = match direction {
                    tiles::DockDirection::Left => (egui_tiles::LinearDir::Horizontal, false),
                    tiles::DockDirection::Right => (egui_tiles::LinearDir::Horizontal, true),
                    tiles::DockDirection::Top => (egui_tiles::LinearDir::Vertical, false),
                    tiles::DockDirection::Bottom => (egui_tiles::LinearDir::Vertical, true),
                };

                // Re-create the source pane.
                let title = state
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
                    &mut state.tile_tree,
                    target_tile,
                    new_pane,
                    linear_dir,
                    insert_after,
                );

                tiles::activate_terminal_tab(&mut state.tile_tree, source);
                // Force both sessions dirty so the new layout renders immediately.
                for sid in [source, target] {
                    if let Some(s) = state.sessions.get_mut(&sid) {
                        s.is_dirty = true;
                    }
                }
                state.scheduler.mark_dirty();
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
                UiAction::DockWebviewLeft => (egui_tiles::LinearDir::Horizontal, false),
                UiAction::DockWebviewDown => (egui_tiles::LinearDir::Vertical, true),
                _ => (egui_tiles::LinearDir::Horizontal, true), // DockWebview / DockWebviewRight
            };
            if let Some(idx) = state.ui_state.active_webview
                && let Some(tab) = state.webview_tabs.get_mut(idx)
            {
                tab.mode = BrowserViewMode::Docked;

                // If the webview was in an overlay panel, reparent it
                // back to the main window and destroy the overlay renderer.
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                let had_overlay = tab.manager.overlay.is_some();
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let had_overlay = false;
                if had_overlay {
                    tab.manager.repaint_webview(&state.window);
                }

                // Move the native webview to zero-size off-screen instead of
                // hiding it, so that the next frame's positioning code can
                // reposition it without a WKWebView show/hide latency flash.
                let offscreen = Bounds::new(0, 0, 1, 1);
                tab.manager.set_bounds(offscreen);
                let wid = tab.id;
                let title = tab.title.clone();
                let url = tab.url.clone();
                state.active_webview = idx;

                // Sync the z-order manager.
                state.webview_manager.demote_to_docked(wid);

                // Hide float panel if no overlay webviews remain after demotion.
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                {
                    let has_overlays = state
                        .webview_tabs
                        .iter()
                        .any(|wt| wt.mode == BrowserViewMode::Overlay && wt.manager.visible);
                    if !has_overlays {
                        state.float_panel_state.clear();
                        if let Some(ref mut fr) = state.float_renderer {
                            use crate::renderer::overlay::Renderer;
                            fr.set_visible(false);
                        }
                    }
                }

                // Find the active terminal tile and split with the webview pane.
                if let Some(active_sid) = tiles::active_terminal_session(&state.tile_tree)
                    && let Some(active_tile) =
                        tiles::find_terminal_tile(&state.tile_tree, active_sid)
                {
                    let webview_pane = tiles::Pane::BrowserView {
                        webview_id: wid,
                        title,
                        url,
                    };
                    tiles::split_pane_dir(
                        &mut state.tile_tree,
                        active_tile,
                        webview_pane,
                        dir,
                        insert_after,
                    );
                    // Re-activate the terminal tab so the split doesn't
                    // switch focus to the newly-docked webview pane.
                    tiles::activate_terminal_tab(&mut state.tile_tree, active_sid);
                    log::info!("Docked webview {} as split {:?}", wid, dir);
                } else {
                    // No active terminal pane — add as tab instead.
                    tiles::add_browser_view_tab(&mut state.tile_tree, wid, &title, &url);
                    log::info!("Docked webview {} as tab (no active terminal)", wid);
                }
                // Force egui repaint and window redraw so the next frame
                // immediately picks up the new tile layout and repositions
                // the native webview.
                state.scheduler.mark_dirty();
                state.scheduler.mark_dirty();
            } else {
                log::warn!(
                    "DockWebview: no active webview (active_webview={:?})",
                    state.ui_state.active_webview
                );
            }
        }
        UiAction::WebviewToTab(req_wid) => {
            // Resolve webview index: if req_wid != 0, look up by ID;
            // otherwise fall back to active_webview.
            let idx = if req_wid != 0 {
                state.webview_tabs.iter().position(|wt| wt.id == req_wid)
            } else {
                state.ui_state.active_webview
            };
            // Behaviour depends on the webview's current mode:
            // - Overlay → Docked tile pane  (existing logic)
            // - Docked  → Overlay tab       (move back to floating overlay)
            if let Some(idx) = idx
                && let Some(tab) = state.webview_tabs.get_mut(idx)
            {
                if tab.mode == BrowserViewMode::Docked {
                    // ── Docked → Overlay ──
                    // Remove from tile tree and switch to overlay mode.
                    let wid = tab.id;
                    tiles::remove_browser_view_pane(&mut state.tile_tree, wid);
                    tab.mode = BrowserViewMode::Overlay;
                    tab.manager.visible = true;
                    state.webview_manager.promote_to_overlay(wid);
                    log::info!("Webview {} moved from split pane to overlay tab", wid);
                    state.scheduler.mark_dirty();
                } else {
                    // ── Overlay → Docked tile ──
                    tab.mode = BrowserViewMode::Docked;

                    // If the webview was in an overlay panel, reparent it
                    // back to the main window and destroy the overlay renderer.
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    let had_overlay = tab.manager.overlay.is_some();
                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                    let had_overlay = false;
                    if had_overlay {
                        tab.manager.repaint_webview(&state.window);
                    }

                    // Move off-screen instead of hiding to avoid WKWebView latency.
                    let offscreen = Bounds::new(0, 0, 1, 1);
                    tab.manager.set_bounds(offscreen);
                    let wid = tab.id;
                    let title = tab.title.clone();
                    let url = tab.url.clone();
                    state.active_webview = idx;

                    // Sync the z-order manager.
                    state.webview_manager.demote_to_docked(wid);

                    // Hide float panel if no overlay webviews remain after tab conversion.
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    {
                        let has_overlays = state
                            .webview_tabs
                            .iter()
                            .any(|wt| wt.mode == BrowserViewMode::Overlay && wt.manager.visible);
                        if !has_overlays {
                            state.float_panel_state.clear();
                            if let Some(ref mut fr) = state.float_renderer {
                                use crate::renderer::overlay::Renderer;
                                fr.set_visible(false);
                            }
                        }
                    }

                    tiles::add_browser_view_tab(&mut state.tile_tree, wid, &title, &url);
                    log::info!("Webview {} moved to tile tab", wid);
                }
            }
        }
        UiAction::OpenInSystemBrowser => {
            // Open the current webview URL in the system default browser.
            let url = if let Some(idx) = state.ui_state.active_webview
                && let Some(tab) = state.webview_tabs.get(idx)
            {
                Some(tab.url.clone())
            } else {
                // Try address bar URL as fallback.
                let u = state.ui_state.address_bar.url.clone();
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
            super::split::handle_split(state, &action);
        }
        _ => {}
    }
}
