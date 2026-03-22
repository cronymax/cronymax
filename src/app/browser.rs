//! Browser lifecycle management (open / close / switch).
//!
//! Data types (`BrowserTab`, `BrowserManager`, `ZLayer`, etc.) live in
//! `ui::browser`.  This module handles the stateful operations that
//! require split borrows (`&mut Ui` + `&mut AppCtx`).

use super::*;
use crate::renderer::panel::LogicalRect;
use crate::ui::model::AppCtx;
use crate::{renderer::viewport::Viewport, ui::Ui};

// Re-export types from ui::browser so existing app/ code still compiles.
pub(super) use crate::ui::browser::{BrowserManager, BrowserTab, ZLayer};

// ─── Webview Management ──────────────────────────────────────────────────────

pub(crate) fn open_browser(
    ui: &mut Ui,
    ctx: &mut AppCtx<'_>,
    url: &str,
    #[allow(unused)] event_loop: &ActiveEventLoop,
) {
    log::info!("Opening browser as overlay: {}", url);

    let active_sid = tiles::active_terminal_session(&ui.tile_tree);
    let active_wid = tiles::active_browser_view_id(&ui.tile_tree);

    let existing_overlay = if let Some(wid) = active_wid {
        ui.browser_tabs
            .iter()
            .position(|wt| wt.mode == BrowserViewMode::Overlay && wt.paired_webview == Some(wid))
    } else if let Some(sid) = active_sid {
        ui.browser_tabs
            .iter()
            .position(|wt| wt.mode == BrowserViewMode::Overlay && wt.paired_session == Some(sid))
    } else {
        None
    };
    if let Some(idx) = existing_overlay {
        log::info!("Reusing existing paired overlay webview");
        ui.browser_tabs[idx].browser.navigate(url);
        ui.browser_tabs[idx].browser.view.set_visible(true);
        ui.active_browser = idx;
        ctx.ui_state.active_browser = Some(idx);
        ctx.scheduler.mark_dirty();
        return;
    }

    let win_size = ui.frame.window.inner_size();
    let scale = ui.frame.window.scale_factor() as f32;

    let logical_w = win_size.width as f32 / scale;
    let logical_h = win_size.height as f32 / scale;
    let address_bar_h = ui.styles.address_bar_height();
    let bw = ui.styles.sizes.border;

    let pop_w = (logical_w * 0.80).min(logical_w - 40.0).max(300.0);
    let pop_h = (logical_h * 0.70).min(logical_h - 80.0).max(200.0);
    let total_w = pop_w + 2.0 * bw;
    let total_h = pop_h + 2.0 * bw;
    let frame_left = (logical_w - total_w) / 2.0;
    let frame_top = (logical_h - total_h) / 2.0;

    let panel_lx = frame_left + bw;
    let panel_ly = frame_top + bw;
    let panel_lw = pop_w;
    let panel_lh = pop_h;

    let content_bounds = Viewport::new(
        (panel_lx * scale).round(),
        ((panel_ly + address_bar_h) * scale).round(),
        (panel_lw * scale).round(),
        ((panel_lh - address_bar_h) * scale).round(),
    );

    #[allow(unused_mut)]
    let mut overlay_modal = None;
    let rect = crate::renderer::panel::LogicalRect {
        x: panel_lx,
        y: panel_ly,
        w: panel_lw,
        h: panel_lh,
        scale,
    };
    let wv_result = (|| -> Result<Webview, String> {
        let modal = crate::ui::overlay::Modal::new(
            &ui.frame.window,
            Some(event_loop),
            &ui.frame.gpu,
            rect,
        )?;
        let wv_bounds = webview_bounds_below_bar(rect, address_bar_h);
        let bv = Webview::new(&modal.panel, url, wv_bounds)?;
        overlay_modal = Some(modal);
        Ok(bv)
    })()
    .or_else(|e| {
        log::warn!("Could not create overlay panel: {e}; falling back to child-of-window");
        Webview::new(&ui.frame.window, url, content_bounds)
    });

    match wv_result {
        Ok(wv) => {
            wv.send_theme(
                &ctx.config.colors.background,
                &ctx.config.colors.foreground,
                "#4fc3f7",
                &ctx.config.font.family,
                ctx.config.font.size,
            );

            let id = ui.next_browser_id;
            ui.next_browser_id += 1;

            let title = url
                .split("//")
                .nth(1)
                .and_then(|h| h.split('/').next())
                .unwrap_or(url)
                .to_string();

            if let Some(prev) = ui.browser_tabs.get_mut(ui.active_browser)
                && prev.mode == BrowserViewMode::Overlay
            {
                prev.browser.view.set_visible(false);
            }

            ui.browser_tabs.push(BrowserTab {
                browser: crate::ui::browser::Browser {
                    id,
                    title,
                    url: url.to_string(),
                    view: wv,
                    address_bar: AddressBarState::new(url),
                },
                mode: BrowserViewMode::Overlay,
                paired_session: if active_wid.is_some() {
                    None
                } else {
                    active_sid
                },
                paired_webview: active_wid,
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                overlay: overlay_modal,
                overlay_origin: (panel_lx, panel_ly),
            });
            ui.active_browser = ui.browser_tabs.len() - 1;
            ctx.ui_state.active_browser = Some(ui.active_browser);

            ui.browser_manager.register(id, ZLayer::Overlay);

            ctx.scheduler.mark_dirty();
        }
        Err(e) => {
            log::error!("Failed to open webview: {}", e);
        }
    }
}

pub(crate) fn close_active_browser(ui: &mut Ui, ctx: &mut AppCtx<'_>) {
    if ui.browser_tabs.is_empty() {
        return;
    }

    let removed_id = ui.browser_tabs[ui.active_browser].browser.id;
    ui.browser_tabs.remove(ui.active_browser);
    ui.browser_manager.unregister(removed_id);

    if ui.browser_tabs.is_empty() {
        ui.split = None;
        ui.active_browser = 0;
        ctx.ui_state.active_browser = None;

        let win_size = ui.frame.window.inner_size();
        let (viewport, cols, rows) = crate::ui::compute_single_pane(
            win_size.width,
            win_size.height,
            &ctx.cell_size,
            &ui.styles,
        );
        ui.viewport = viewport;
        for session in ctx.sessions.values_mut() {
            session.resize(cols, rows);
        }
        log::info!("All webviews closed, restored full terminal layout");
    } else {
        if ui.active_browser >= ui.browser_tabs.len() {
            ui.active_browser = ui.browser_tabs.len() - 1;
        }
        if let Some(tab) = ui.browser_tabs.get_mut(ui.active_browser) {
            tab.browser.view.set_visible(true);
            let win_size = ui.frame.window.inner_size();
            if let Some(ref split) = ui.split {
                let bounds = split.webview_bounds(win_size.width, win_size.height, &ui.styles);
                tab.browser.view.set_viewport(bounds);
            }
        }
    }

    ctx.scheduler.mark_dirty();
}

pub(crate) fn switch_browser_tab(ui: &mut Ui, ctx: &mut AppCtx<'_>, index: usize) {
    if index >= ui.browser_tabs.len() {
        return;
    }

    if let Some(current) = ui.browser_tabs.get_mut(ui.active_browser) {
        current.browser.view.set_visible(false);
    }

    ui.active_browser = index;
    ctx.ui_state.active_browser = Some(index);

    if let Some(tab) = ui.browser_tabs.get_mut(index) {
        tab.browser.view.set_visible(true);

        if tab.mode == BrowserViewMode::Docked {
            tiles::activate_browser_view_tab(&mut ui.tile_tree, tab.browser.id);
        }

        let win_size = ui.frame.window.inner_size();
        if let Some(ref split) = ui.split {
            tab.browser.view.set_viewport(split.webview_bounds(
                win_size.width,
                win_size.height,
                &ui.styles,
            ));
        }
    }

    ctx.scheduler.mark_dirty();
}

// ─── Overlay helpers ─────────────────────────────────────────────────────────

/// Compute webview bounds within an overlay panel, offsetting below the
/// browser address bar.
pub(crate) fn webview_bounds_below_bar(rect: LogicalRect, browser_height: f32) -> Viewport {
    let phys_w = (rect.w * rect.scale).round();
    let total_phys_h = (rect.h * rect.scale).round();
    let browser_phys_h = (browser_height * rect.scale).round();
    let wv_phys_h = total_phys_h - browser_phys_h;
    Viewport::new(0.0, browser_phys_h, phys_w, wv_phys_h)
}
