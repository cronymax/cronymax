//! Mouse click handling — hit testing and cursor movement.
//!
//! Moved from `app/mouse.rs` as part of the UI-layer split. These
//! functions implement pure UI interaction logic (hit testing tab bars,
//! address bars, terminal panes) and translate clicks into navigation or
//! cursor-movement actions.

use crate::app::browser::{close_active_browser, switch_browser_tab};
use crate::app::AppState;
use crate::renderer::scheduler::RenderSchedule;
use crate::ui::browser::{AddrBarButton, BrowserView};
use crate::ui::tiles;

// ─── Mouse Click Handler ─────────────────────────────────────────────────────

pub(crate) fn handle_mouse_click(state: &mut AppState) {
    let mx = state.ui.mouse_x;
    let my = state.ui.mouse_y;
    let win_size = state.ui.frame.window.inner_size();
    let sw = win_size.width as f32;

    // 1. Terminal tab bar (top, full width, tab_bar_height pixels).
    if (0.0..state.ui.styles.tab_bar_height()).contains(&my) {
        let session_ids = tiles::terminal_session_ids(&state.ui.tile_tree);
        let num_tabs = session_ids.len();
        if let Some(idx) = BrowserView::terminal_tab_hit(mx, my, num_tabs, sw) {
            let sid = session_ids[idx];
            tiles::activate_terminal_tab(&mut state.ui.tile_tree, sid);
            log::info!("Switched to terminal tab: session {}", sid);
            state.scheduler.mark_dirty();
        }
        return;
    }

    // 2. Address bar area (if webview tabs exist).
    if !state.ui.browser_tabs.is_empty()
        && let Some(ref split) = state.ui.split
    {
        let (bar_x, bar_y, bar_w) = split.address_bar_area(win_size.width, &state.ui.styles);
        let lx = mx - bar_x;
        let ly = my - bar_y;
        let bar_h = state.ui.styles.address_bar_height();
        if lx >= 0.0 && lx <= bar_w && (0.0..=bar_h).contains(&ly) {
            // Check close button (rightmost).
            let close_btn_size = bar_h - 8.0;
            let close_x = bar_w - close_btn_size - state.ui.styles.spacing.medium;
            if lx >= close_x && lx <= close_x + close_btn_size {
                log::info!("Close webview button clicked");
                {
                    let (ui, mut ctx) = state.split_ui();
                    close_active_browser(ui, &mut ctx);
                }
                return;
            }

            if let Some(btn) = BrowserView::address_bar_hit(lx, ly, bar_w, &state.ui.styles) {
                match btn {
                    AddrBarButton::Back => {
                        log::info!("Address bar: Back");
                        if let Some(tab) = state.ui.browser_tabs.get(state.ui.active_browser) {
                            tab.browser.go_back();
                        }
                    }
                    AddrBarButton::Forward => {
                        log::info!("Address bar: Forward");
                        if let Some(tab) = state.ui.browser_tabs.get(state.ui.active_browser) {
                            tab.browser.go_forward();
                        }
                    }
                    AddrBarButton::Refresh => {
                        log::info!("Address bar: Refresh");
                        if let Some(tab) = state.ui.browser_tabs.get(state.ui.active_browser) {
                            tab.browser.refresh();
                        }
                    }
                    AddrBarButton::UrlField => {
                        log::info!("Address bar: Edit URL");
                        if let Some(tab) = state.ui.browser_tabs.get_mut(state.ui.active_browser) {
                            tab.browser.address_bar.start_editing();
                        }
                    }
                }
            }
            state.scheduler.mark_dirty();
            return;
        }

        // 3. Browser view tab strip.
        let (strip_x, strip_y, strip_h) =
            split.browser_view_tab_area(win_size.width, win_size.height, &state.ui.styles);
        let lx = mx - strip_x;
        let ly = my - strip_y;
        if (0.0..=state.ui.styles.browser_view_tab_width()).contains(&lx)
            && ly >= 0.0
            && ly <= strip_h
        {
            let num = state.ui.browser_tabs.len();
            if let Some(idx) = BrowserView::browser_view_tab_hit(lx, ly, num, &state.ui.styles)
                && idx != state.ui.active_browser
            {
                log::info!("Switching to webview tab {}", idx);
                {
                    let (ui, mut ctx) = state.split_ui();
                    switch_browser_tab(ui, &mut ctx, idx);
                }
            }
            return;
        }
    }

    // 4. Click in the terminal viewport → start text selection.
    let scale = state.ui.frame.window.scale_factor() as f32;
    for tr in &state.ui.tile_rects {
        if let tiles::TileRect::Terminal { session_id, rect } = tr {
            let px = rect.left() * scale;
            let py = rect.top() * scale;
            let pw = rect.width() * scale;
            let ph = rect.height() * scale;
            if mx >= px && mx < px + pw && my >= py && my < py + ph {
                tiles::activate_terminal_tab(&mut state.ui.tile_tree, *session_id);
                let cell_w = state.ui.frame.terminal.cell_size.width;
                let cell_h = state.ui.frame.terminal.cell_size.height;
                let click_col = ((mx - px) / cell_w) as usize;
                let click_row = ((my - py) / cell_h) as usize;
                state.ui.terminal_selection =
                    Some(crate::ui::TerminalSelection {
                        session_id: *session_id,
                        start_col: click_col,
                        start_row: click_row,
                        end_col: click_col,
                        end_row: click_row,
                    });
                state.ui.selection_dragging = true;
                state.scheduler.mark_dirty();
                return;
            }
        }
    }
    // Fallback: use global viewport.
    let vp = &state.ui.viewport;
    if mx >= vp.x && mx < vp.x + vp.width && my >= vp.y && my < vp.y + vp.height {
        let cell_w = state.ui.frame.terminal.cell_size.width;
        let cell_h = state.ui.frame.terminal.cell_size.height;
        let click_col = ((mx - vp.x) / cell_w) as usize;
        let click_row = ((my - vp.y) / cell_h) as usize;
        if let Some(sid) = tiles::active_terminal_session(&state.ui.tile_tree) {
            state.ui.terminal_selection =
                Some(crate::ui::TerminalSelection {
                    session_id: sid,
                    start_col: click_col,
                    start_row: click_row,
                    end_col: click_col,
                    end_row: click_row,
                });
            state.ui.selection_dragging = true;
        }
        state.scheduler.mark_dirty();
    }
}
