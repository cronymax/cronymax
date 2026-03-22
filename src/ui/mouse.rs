//! Mouse click handling — hit testing and cursor movement.
//!
//! Moved from `app/mouse.rs` as part of the UI-layer split. These
//! functions implement pure UI interaction logic (hit testing tab bars,
//! address bars, terminal panes) and translate clicks into navigation or
//! cursor-movement actions.

use crate::renderer::scheduler::RenderSchedule;
use crate::ui::browser::{AddrBarButton, BrowserView};
use crate::ui::model::AppCtx;
use crate::ui::{Ui, tiles};

// ─── Mouse Click Handler ─────────────────────────────────────────────────────
impl Ui {
    pub fn handle_mouse_click(&mut self, ctx: &mut AppCtx<'_>) {
        let mx = self.mouse_x;
        let my = self.mouse_y;
        let win_size = self.frame.window.inner_size();
        let sw = win_size.width as f32;

        // 1. Terminal tab bar (top, full width, tab_bar_height pixels).
        if (0.0..self.styles.tab_bar_height()).contains(&my) {
            let session_ids = tiles::terminal_session_ids(&self.tile_tree);
            let num_tabs = session_ids.len();
            if let Some(idx) = BrowserView::terminal_tab_hit(mx, my, num_tabs, sw) {
                let sid = session_ids[idx];
                tiles::activate_terminal_tab(&mut self.tile_tree, sid);
                log::info!("Switched to terminal tab: session {}", sid);
                ctx.scheduler.mark_dirty();
            }
            return;
        }

        // 2. Address bar area (if webview tabs exist).
        if !self.browser_tabs.is_empty()
            && let Some(ref split) = self.split
        {
            let (bar_x, bar_y, bar_w) = split.address_bar_area(win_size.width, &self.styles);
            let lx = mx - bar_x;
            let ly = my - bar_y;
            let bar_h = self.styles.address_bar_height();
            if lx >= 0.0 && lx <= bar_w && (0.0..=bar_h).contains(&ly) {
                // Check close button (rightmost).
                let close_btn_size = bar_h - 8.0;
                let close_x = bar_w - close_btn_size - self.styles.spacing.medium;
                if lx >= close_x && lx <= close_x + close_btn_size {
                    log::info!("Close webview button clicked");
                    {
                        self.close_active_browser(ctx);
                    }
                    return;
                }

                if let Some(btn) = BrowserView::address_bar_hit(lx, ly, bar_w, &self.styles) {
                    match btn {
                        AddrBarButton::Back => {
                            log::info!("Address bar: Back");
                            if let Some(tab) = self.browser_tabs.get(self.active_browser) {
                                tab.browser.go_back();
                            }
                        }
                        AddrBarButton::Forward => {
                            log::info!("Address bar: Forward");
                            if let Some(tab) = self.browser_tabs.get(self.active_browser) {
                                tab.browser.go_forward();
                            }
                        }
                        AddrBarButton::Refresh => {
                            log::info!("Address bar: Refresh");
                            if let Some(tab) = self.browser_tabs.get(self.active_browser) {
                                tab.browser.refresh();
                            }
                        }
                        AddrBarButton::UrlField => {
                            log::info!("Address bar: Edit URL");
                            if let Some(tab) = self.browser_tabs.get_mut(self.active_browser) {
                                tab.browser.address_bar.start_editing();
                            }
                        }
                    }
                }
                ctx.scheduler.mark_dirty();
                return;
            }

            // 3. Browser view tab strip.
            let (strip_x, strip_y, strip_h) =
                split.browser_view_tab_area(win_size.width, win_size.height, &self.styles);
            let lx = mx - strip_x;
            let ly = my - strip_y;
            if (0.0..=self.styles.browser_view_tab_width()).contains(&lx)
                && ly >= 0.0
                && ly <= strip_h
            {
                let num = self.browser_tabs.len();
                if let Some(idx) = BrowserView::browser_view_tab_hit(lx, ly, num, &self.styles)
                    && idx != self.active_browser
                {
                    log::info!("Switching to webview tab {}", idx);
                    {
                        self.switch_browser_tab(ctx, idx);
                    }
                }
                return;
            }
        }

        // 4. Click in the terminal viewport → start text selection.
        let scale = self.frame.window.scale_factor() as f32;
        let pad = self.styles.spacing.medium * scale;
        for tr in &self.tile_rects {
            if let tiles::TileRect::Terminal { session_id, rect } = tr {
                let px = rect.left() * scale;
                let py = rect.top() * scale;
                let pw = rect.width() * scale;
                let ph = rect.height() * scale;
                if mx >= px && mx < px + pw && my >= py && my < py + ph {
                    tiles::activate_terminal_tab(&mut self.tile_tree, *session_id);
                    let cell_w = self.frame.terminal.cell_size.width * scale;
                    let cell_h = self.frame.terminal.cell_size.height * scale;
                    let click_col = ((mx - px - pad).max(0.0) / cell_w) as usize;
                    let click_row = ((my - py - pad).max(0.0) / cell_h) as usize;
                    self.terminal_selection = Some(crate::ui::TerminalSelection {
                        session_id: *session_id,
                        start_col: click_col,
                        start_row: click_row,
                        end_col: click_col,
                        end_row: click_row,
                    });
                    self.selection_dragging = true;
                    ctx.scheduler.mark_dirty();
                    return;
                }
            }
        }
        // Fallback: use global viewport.
        let vp = &self.viewport;
        if mx >= vp.x && mx < vp.x + vp.width && my >= vp.y && my < vp.y + vp.height {
            let cell_w = self.frame.terminal.cell_size.width * scale;
            let cell_h = self.frame.terminal.cell_size.height * scale;
            let click_col = ((mx - vp.x) / cell_w) as usize;
            let click_row = ((my - vp.y) / cell_h) as usize;
            if let Some(sid) = tiles::active_terminal_session(&self.tile_tree) {
                self.terminal_selection = Some(crate::ui::TerminalSelection {
                    session_id: sid,
                    start_col: click_col,
                    start_row: click_row,
                    end_col: click_col,
                    end_row: click_row,
                });
                self.selection_dragging = true;
            }
            ctx.scheduler.mark_dirty();
        }
    }
}
