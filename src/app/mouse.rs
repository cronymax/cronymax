//! Mouse click handling extracted from keybinds.rs

use super::*;

// ─── Mouse Click Handler ─────────────────────────────────────────────────────

pub(super) fn handle_mouse_click(state: &mut AppState) {
    let mx = state.mouse_x;
    let my = state.mouse_y;
    let win_size = state.window.inner_size();
    let sw = win_size.width as f32;

    // 1. Terminal tab bar (top, full width, tab_bar_height pixels).
    if (0.0..state.styles.tab_bar_height()).contains(&my) {
        let session_ids = tiles::terminal_session_ids(&state.tile_tree);
        let num_tabs = session_ids.len();
        if let Some(idx) = browser::BrowserOverlay::terminal_tab_hit(mx, my, num_tabs, sw) {
            let sid = session_ids[idx];
            tiles::activate_terminal_tab(&mut state.tile_tree, sid);
            log::info!("Switched to terminal tab: session {}", sid);
            state.scheduler.mark_dirty();
        }
        return;
    }

    // 2. Address bar area (if webview tabs exist).
    if !state.webview_tabs.is_empty()
        && let Some(ref split) = state.split
    {
        let (bar_x, bar_y, bar_w) = split.address_bar_area(win_size.width, &state.styles);
        let lx = mx - bar_x;
        let ly = my - bar_y;
        let bar_h = state.styles.address_bar_height();
        if lx >= 0.0 && lx <= bar_w && (0.0..=bar_h).contains(&ly) {
            // Check close button (rightmost).
            let close_btn_size = bar_h - 8.0;
            let close_x = bar_w - close_btn_size - state.styles.spacing.medium;
            if lx >= close_x && lx <= close_x + close_btn_size {
                log::info!("Close webview button clicked");
                close_active_webview(state);
                return;
            }

            if let Some(btn) =
                browser::BrowserOverlay::address_bar_hit(lx, ly, bar_w, &state.styles)
            {
                match btn {
                    AddrBarButton::Back => {
                        log::info!("Address bar: Back");
                        if let Some(tab) = state.webview_tabs.get(state.active_webview) {
                            let _ = tab.manager.webview.evaluate_script("window.history.back()");
                        }
                    }
                    AddrBarButton::Forward => {
                        log::info!("Address bar: Forward");
                        if let Some(tab) = state.webview_tabs.get(state.active_webview) {
                            let _ = tab
                                .manager
                                .webview
                                .evaluate_script("window.history.forward()");
                        }
                    }
                    AddrBarButton::Refresh => {
                        log::info!("Address bar: Refresh");
                        if let Some(tab) = state.webview_tabs.get_mut(state.active_webview) {
                            let url = tab.url.clone();
                            tab.manager.navigate(&url);
                        }
                    }
                    AddrBarButton::UrlField => {
                        log::info!("Address bar: Edit URL");
                        if let Some(tab) = state.webview_tabs.get_mut(state.active_webview) {
                            tab.address_bar.start_editing();
                        }
                    }
                }
            }
            state.scheduler.mark_dirty();
            return;
        }

        // 3. Browser view tab strip.
        let (strip_x, strip_y, strip_h) =
            split.browser_view_tab_area(win_size.width, win_size.height, &state.styles);
        let lx = mx - strip_x;
        let ly = my - strip_y;
        if (0.0..=state.styles.browser_view_tab_width()).contains(&lx) && ly >= 0.0 && ly <= strip_h
        {
            let num = state.webview_tabs.len();
            if let Some(idx) =
                browser::BrowserOverlay::browser_view_tab_hit(lx, ly, num, &state.styles)
                && idx != state.active_webview
            {
                log::info!("Switching to webview tab {}", idx);
                switch_webview_tab(state, idx);
            }
            return;
        }
    }

    // 4. Click in the terminal viewport → determine pane and move cursor.
    let scale = state.window.scale_factor() as f32;
    for tr in &state.tile_rects {
        if let tiles::TileRect::Terminal { session_id, rect } = tr {
            let px = rect.left() * scale;
            let py = rect.top() * scale;
            let pw = rect.width() * scale;
            let ph = rect.height() * scale;
            if mx >= px && mx < px + pw && my >= py && my < py + ph {
                // Activate this pane's session.
                tiles::activate_terminal_tab(&mut state.tile_tree, *session_id);
                let cell_w = state.renderer.cell_size.width;
                let cell_h = state.renderer.cell_size.height;
                let click_col = ((mx - px) / cell_w) as usize;
                let click_row = ((my - py) / cell_h) as usize;
                move_cursor_to(state, click_col, click_row);
                state.scheduler.mark_dirty();
                return;
            }
        }
    }
    // Fallback: use global viewport.
    let vp = &state.viewport;
    if mx >= vp.x && mx < vp.x + vp.width && my >= vp.y && my < vp.y + vp.height {
        let cell_w = state.renderer.cell_size.width;
        let cell_h = state.renderer.cell_size.height;
        let click_col = ((mx - vp.x) / cell_w) as usize;
        let click_row = ((my - vp.y) / cell_h) as usize;
        move_cursor_to(state, click_col, click_row);
        state.scheduler.mark_dirty();
    }
}

/// Move the terminal cursor to (target_col, target_row) by emitting arrow key
/// escape sequences. This works for shell prompts and readline-based input.
pub(super) fn move_cursor_to(state: &mut AppState, target_col: usize, target_row: usize) {
    let active_sid = match tiles::active_terminal_session(&state.tile_tree) {
        Some(id) => id,
        None => return,
    };

    let (cur_col, cur_row) = {
        let session = match state.sessions.get(&active_sid) {
            Some(s) => s,
            None => return,
        };
        let term = session.state.term();
        let cursor = &term.grid().cursor;
        (cursor.point.column.0, cursor.point.line.0 as usize)
    };

    // Calculate delta in rows and columns.
    let row_delta = target_row as i32 - cur_row as i32;
    let col_delta = target_col as i32 - cur_col as i32;

    // Build escape sequences.
    // Arrow Up = \x1b[A, Down = \x1b[B, Right = \x1b[C, Left = \x1b[D
    let mut seq = Vec::new();

    if row_delta > 0 {
        for _ in 0..row_delta {
            seq.extend_from_slice(b"\x1b[B");
        }
    } else if row_delta < 0 {
        for _ in 0..(-row_delta) {
            seq.extend_from_slice(b"\x1b[A");
        }
    }

    if col_delta > 0 {
        for _ in 0..col_delta {
            seq.extend_from_slice(b"\x1b[C");
        }
    } else if col_delta < 0 {
        for _ in 0..(-col_delta) {
            seq.extend_from_slice(b"\x1b[D");
        }
    }

    if !seq.is_empty()
        && let Some(session) = state.sessions.get_mut(&active_sid)
    {
        session.write_to_pty(&seq);
    }
}
