//! UI-state synchronization — derives the compact `UiState` representation
//! from the authoritative `AppState` each frame.
//!
//! Moved from `app/tabs.rs` as part of the UI-layer split.  This is pure
//! state-projection logic with no lifecycle / window management concerns.

use crate::app::AppState;
use crate::renderer::terminal::SessionId;
use crate::ui::tiles;
use crate::ui::types::{BrowserViewMode, TabInfo, TerminalInfo};

/// Synchronize `state.ui_state` from the authoritative `AppState` fields.
///
/// Called once per frame before painting to keep the lightweight `UiState`
/// snapshot in sync with the heavier `AppState`.
pub(crate) fn sync_ui_state(state: &mut AppState, active_sid: SessionId) {
    // Update terminal tab titles and ensure the tab list is in sync.
    for tab_entry in state.ui_state.tabs.iter_mut() {
        if let TabInfo::Chat {
            session_id, title, ..
        }
        | TabInfo::Terminal {
            session_id, title, ..
        } = tab_entry
            && let Some(session) = state.sessions.get(session_id)
            && *title != session.title
        {
            *title = session.title.clone();
            tiles::update_terminal_title(&mut state.ui.tile_tree, *session_id, &session.title);
        }
    }

    // Sync active tab index from tile_tree.
    if let Some(active) = tiles::active_terminal_session(&state.ui.tile_tree)
        && let Some(idx) = state
            .ui_state
            .tabs
            .iter()
            .position(|t| matches!(t, TabInfo::Chat { session_id: s, .. } | TabInfo::Terminal { session_id: s, .. } if *s == active))
    {
        state.ui_state.active_tab = idx;
    }

    // Rebuild browser view entries in the unified tabs list.
    // Retain terminal and channel entries; rebuild browser view entries from webview_tabs.
    state.ui_state.tabs.retain(|t| !t.is_browser_view());
    for wt in &state.ui.browser_tabs {
        state.ui_state.tabs.push(TabInfo::BrowserView {
            webview_id: wt.browser.id,
            title: wt.browser.title.clone(),
            url: wt.browser.url.clone(),
            mode: wt.mode,
        });
    }

    // Sync active_webview from tile tree (handles egui_tiles native tab switching).
    if let Some(wid) = tiles::active_browser_view_id(&state.ui.tile_tree)
        && let Some(idx) = state
            .ui
            .browser_tabs
            .iter()
            .position(|w| w.browser.id == wid)
    {
        state.ui.active_browser = idx;
        state.ui_state.active_browser = Some(idx);
    }
    // If no webview is active in the tile tree, leave active_webview as-is so
    // overlay/docked webview selection is preserved.

    // Derive active_webview_id from active_webview index for UI lookups.
    state.ui_state.active_browser_id = state
        .ui_state
        .active_browser
        .and_then(|idx| state.ui.browser_tabs.get(idx))
        .map(|wt| wt.browser.id);

    // Per-frame webview visibility sync: show/hide overlay webviews based on
    // their pairing (with a terminal session OR a docked webview tab).
    let active_terminal_sid = tiles::active_terminal_session(&state.ui.tile_tree);
    let active_wid = tiles::active_browser_view_id(&state.ui.tile_tree);
    {
        let mut best_overlay: Option<usize> = None;
        for (i, wt) in state.ui.browser_tabs.iter_mut().enumerate() {
            if wt.mode == BrowserViewMode::Overlay {
                // Determine if this overlay should be visible:
                // - Paired with the active terminal session, OR
                // - Paired with the active docked webview tab.
                let should_show = wt
                    .paired_session
                    .is_some_and(|sid| active_terminal_sid == Some(sid))
                    || wt.paired_webview.is_some_and(|wid| active_wid == Some(wid));
                if wt.browser.view.visible != should_show {
                    wt.browser.view.set_visible(should_show);
                }
                if should_show && best_overlay.is_none() {
                    best_overlay = Some(i);
                }
            }
        }
        // Restore/clear active_webview based on pairing.
        if let Some(idx) = best_overlay {
            state.ui.active_browser = idx;
            state.ui_state.active_browser = Some(idx);
        } else {
            // No overlay is paired with the active tab.
            // Clear active_webview if it was pointing to a overlay so
            // the overlay frame is not drawn (which would block clicks
            // on widgets underneath, like chat hyperlinks).
            if let Some(aw) = state.ui_state.active_browser
                && state
                    .ui
                    .browser_tabs
                    .get(aw)
                    .is_some_and(|wt| wt.mode == BrowserViewMode::Overlay)
            {
                state.ui_state.active_browser = None;
            }
        }
    }

    // Sync address bar URL from active webview (only when not editing).
    // Only sync from overlay webviews — docked webviews manage their own
    // per-pane URLs via the Behavior::webview_urls map.
    if !state.ui_state.address_bar.editing
        && let Some(idx) = state.ui_state.active_browser
        && let Some(wt) = state.ui.browser_tabs.get(idx)
        && wt.mode == BrowserViewMode::Overlay
    {
        state.ui_state.address_bar.url = wt.browser.url.clone();
    }

    // Ensure colon_buf info is reflected.
    let _ = active_sid;

    // ── Profile picker sync ───────────────────────────────────────────────
    {
        let mgr = state.profile_manager.lock().unwrap();
        let list: Vec<(String, String)> = mgr
            .list()
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        let active_id = mgr.active().map(|p| p.id.clone()).unwrap_or_default();
        state.ui_state.profile_list = list;
        state.ui_state.active_profile_id = active_id;
    }

    // ── Sync shared skill state ──────────────────────────────────────────
    // Update shared tab info for AI skill queries.
    if let Ok(mut shared_tabs) = state.shared_tab_info.lock() {
        shared_tabs.clone_from(&state.ui_state.tabs);
    }

    // Update shared terminal info for AI skill queries.
    if let Ok(mut shared_terms) = state.shared_terminal_info.lock() {
        let infos: Vec<TerminalInfo> = state
            .sessions
            .iter()
            .map(|(sid, session)| TerminalInfo {
                session_id: *sid,
                title: session.title.clone(),
                pid: session.child_pid,
                cwd: session.cwd.clone(),
                running: !session.exited,
            })
            .collect();
        *shared_terms = infos;
    }
}
