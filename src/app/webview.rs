//! Sub-module extracted from app/mod.rs

use super::*;

// ─── Webview Management ──────────────────────────────────────────────────────

pub(super) fn open_webview(
    state: &mut AppState,
    url: &str,
    #[allow(unused)] event_loop: &ActiveEventLoop,
) {
    log::info!("Opening webview as overlay: {}", url);

    let active_sid = tiles::active_terminal_session(&state.tile_tree);
    let active_wid = tiles::active_browser_view_id(&state.tile_tree);

    // If there's already a overlay webview paired with the active terminal
    // session (or the active docked webview), navigate it instead of creating
    // a new one.
    // When a docked webview is the active tile, look for a overlay paired
    // with that webview.  Otherwise look for one paired with the terminal.
    let existing_overlay =
        if let Some(wid) = active_wid {
            state.webview_tabs.iter().position(|wt| {
                wt.mode == BrowserViewMode::Overlay && wt.paired_webview == Some(wid)
            })
        } else if let Some(sid) = active_sid {
            state.webview_tabs.iter().position(|wt| {
                wt.mode == BrowserViewMode::Overlay && wt.paired_session == Some(sid)
            })
        } else {
            None
        };
    if let Some(idx) = existing_overlay {
        log::info!("Reusing existing paired overlay webview");
        state.webview_tabs[idx].manager.navigate(url);
        state.webview_tabs[idx].url = url.to_string();
        state.webview_tabs[idx].address_bar.update_url(url);
        state.webview_tabs[idx].manager.set_visible(true);
        state.active_webview = idx;
        state.ui_state.active_webview = Some(idx);
        state.scheduler.mark_dirty();
        return;
    }

    let win_size = state.window.inner_size();
    let scale = state.window.scale_factor() as f32;

    // Compute overlay bounds (centered, 80% of window).
    let logical_w = win_size.width as f32 / scale;
    let logical_h = win_size.height as f32 / scale;
    let address_bar_h = state.styles.address_bar_height();
    let bw = state.styles.sizes.border;

    // Overlay dimensions (content size inside borders).
    let pop_w = (logical_w * 0.80).min(logical_w - 40.0).max(300.0);
    let pop_h = (logical_h * 0.70).min(logical_h - 80.0).max(200.0);
    let total_w = pop_w + 2.0 * bw;
    let total_h = pop_h + 2.0 * bw;
    let frame_left = (logical_w - total_w) / 2.0;
    let frame_top = (logical_h - total_h) / 2.0;

    // Panel coords: inside borders, full height (including address bar).
    let panel_lx = frame_left + bw;
    let panel_ly = frame_top + bw;
    let panel_lw = pop_w;
    let panel_lh = pop_h;

    // Content bounds for the fallback (child-of-window) path.
    let content_bounds = Bounds::new(
        (panel_lx * scale).round() as u32,
        ((panel_ly + address_bar_h) * scale).round() as u32,
        (panel_lw * scale).round() as u32,
        ((panel_lh - address_bar_h) * scale).round() as u32,
    );

    // Create the overlay webview inside a platform child panel (NSPanel on
    // macOS, owned popup on Windows).  The panel covers the full overlay
    // frame (address bar + content); the webview is offset below the
    // browser/address-bar area.  Falls back to a plain child-of-window
    // webview on platforms that don't support child windows.
    let wv_result = BrowserView::new_overlay(&crate::webview::OverlayConfig {
        parent: &state.window,
        event_loop: Some(event_loop),
        url,
        lx: panel_lx,
        ly: panel_ly,
        lw: panel_lw,
        lh: panel_lh,
        scale,
        browser_height: address_bar_h,
        gpu: &state.gpu,
    })
    .or_else(|e| {
        log::warn!("Could not create overlay panel: {e}; falling back to child-of-window");
        BrowserView::new(&state.window, url, content_bounds)
    });

    match wv_result {
        Ok(wv) => {
            wv.send_theme(
                &state.config.colors.background,
                &state.config.colors.foreground,
                "#4fc3f7", // accent color
                &state.config.font.family,
                state.config.font.size,
            );

            let id = state.next_webview_id;
            state.next_webview_id += 1;

            let title = url
                .split("//")
                .nth(1)
                .and_then(|h| h.split('/').next())
                .unwrap_or(url)
                .to_string();

            // Hide previously active *overlay* webview.
            // Docked webviews stay visible — the tile positioning code
            // manages their visibility in the RedrawRequested handler.
            if let Some(prev) = state.webview_tabs.get_mut(state.active_webview)
                && prev.mode == BrowserViewMode::Overlay
            {
                prev.manager.set_visible(false);
            }

            state.webview_tabs.push(WebviewTab {
                id,
                title,
                url: url.to_string(),
                manager: wv,
                address_bar: AddressBarState::new(url),
                mode: BrowserViewMode::Overlay,
                // Only pair with a terminal session if the terminal is
                // truly the active tile (not via the fallback in
                // active_terminal_session). When the active tile is a
                // docked webview we pair with that webview instead.
                paired_session: if active_wid.is_some() {
                    None
                } else {
                    active_sid
                },
                paired_webview: active_wid,
            });
            state.active_webview = state.webview_tabs.len() - 1;
            state.ui_state.active_webview = Some(state.active_webview);

            // Register with the z-order manager.
            state.webview_manager.register(id, ZLayer::Overlay);

            // In overlay mode, don't resize the terminal — it stays full size.
            state.scheduler.mark_dirty();
        }
        Err(e) => {
            log::error!("Failed to open webview: {}", e);
        }
    }
}

pub(super) fn close_active_webview(state: &mut AppState) {
    if state.webview_tabs.is_empty() {
        return;
    }

    let removed_id = state.webview_tabs[state.active_webview].id;
    state.webview_tabs.remove(state.active_webview);
    state.webview_manager.unregister(removed_id);

    if state.webview_tabs.is_empty() {
        state.split = None;
        state.active_webview = 0;
        state.ui_state.active_webview = None;

        let win_size = state.window.inner_size();
        let (viewport, cols, rows) = ui::compute_single_pane(
            win_size.width,
            win_size.height,
            &state.renderer.cell_size,
            &state.styles,
        );
        state.viewport = viewport;
        for session in state.sessions.values_mut() {
            session.resize(cols, rows);
        }
        log::info!("All webviews closed, restored full terminal layout");
    } else {
        if state.active_webview >= state.webview_tabs.len() {
            state.active_webview = state.webview_tabs.len() - 1;
        }
        if let Some(tab) = state.webview_tabs.get_mut(state.active_webview) {
            tab.manager.set_visible(true);
            let win_size = state.window.inner_size();
            if let Some(ref split) = state.split {
                let bounds = split.webview_bounds(win_size.width, win_size.height, &state.styles);
                tab.manager.set_bounds(bounds);
            }
        }
    }

    state.scheduler.mark_dirty();
}

pub(super) fn switch_webview_tab(state: &mut AppState, index: usize) {
    if index >= state.webview_tabs.len() {
        return;
    }

    // Hide the previously active webview.
    if let Some(current) = state.webview_tabs.get_mut(state.active_webview) {
        current.manager.set_visible(false);
    }

    state.active_webview = index;
    state.ui_state.active_webview = Some(index);

    if let Some(tab) = state.webview_tabs.get_mut(index) {
        tab.manager.set_visible(true);

        // If webview is docked in the tile tree, activate it there too.
        if tab.mode == BrowserViewMode::Docked {
            tiles::activate_browser_view_tab(&mut state.tile_tree, tab.id);
        }

        // Position the webview (docked webviews will be repositioned from pane_ui next frame).
        let win_size = state.window.inner_size();
        if let Some(ref split) = state.split {
            let bounds = split.webview_bounds(win_size.width, win_size.height, &state.styles);
            tab.manager.set_bounds(bounds);
        }
    }

    state.scheduler.mark_dirty();
}
