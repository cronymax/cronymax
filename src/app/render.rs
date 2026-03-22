//! Render loop and about_to_wait handler extracted from app/mod.rs

use std::time::{Duration, Instant};

use crate::renderer::scheduler::RenderSchedule;

use super::*;

pub(super) fn handle_about_to_wait(app: &mut App, event_loop: &ActiveEventLoop) {
    if let Some(state) = app.state.as_mut() {
        // 1. Drain any pending PTY data.
        for session in state.sessions.values_mut() {
            if session.process_pty_output() {
                state.scheduler.mark_dirty();
            }
        }

        // 2. (Deferred repaint deadlines are now handled inside the scheduler.)

        // 3. Check cursor blink timer.
        if state.next_cursor_blink.is_some_and(|t| Instant::now() >= t) {
            state.cursor_visible = !state.cursor_visible;
            state.next_cursor_blink = Some(Instant::now() + Duration::from_millis(530));
            state.scheduler.mark_dirty();
        }

        // 4. Process webview IPC messages from all tabs.
        let mut new_window_urls: Vec<String> = Vec::new();
        let mut title_updates: Vec<(u32, String)> = Vec::new();
        let mut url_updates: Vec<(u32, String)> = Vec::new();
        for tab in &mut state.ui.browser_tabs {
            for msg in tab.browser.view.process_ipc() {
                match msg {
                    WebviewToRust::TerminalInput { payload } => {
                        if let Some(sid) = tiles::active_terminal_session(&state.ui.tile_tree)
                            && let Some(session) = state.sessions.get_mut(&sid)
                        {
                            session.write_to_pty(payload.data.as_bytes());
                        }
                    }
                    WebviewToRust::NavigateRequest { payload } => {
                        tab.browser.navigate(&payload.url);
                        url_updates.push((tab.browser.id, payload.url.clone()));
                    }
                    WebviewToRust::Close => {}
                    WebviewToRust::ScriptResult { payload } => {
                        // Route script result to the pending results map.
                        log::info!(
                            "ScriptResult[{}]: result={:?}, error={:?}",
                            payload.request_id,
                            payload.result.as_deref().map(|s| &s[..s.len().min(100)]),
                            payload.error,
                        );
                        if let Ok(mut map) = state.pending_results.lock()
                            && let Some(sender) = map.remove(&payload.request_id)
                        {
                            let value = if let Some(err) = payload.error {
                                serde_json::json!({ "error": err })
                            } else {
                                serde_json::json!({
                                    "result": payload.result.unwrap_or_default()
                                })
                            };
                            let _ = sender.send(value);
                        }
                    }
                    _ => {}
                }
            }
            // Drain window.open() / target="_blank" requests.
            new_window_urls.extend(tab.browser.view.drain_new_window_urls());
            // Drain document title changes from the webview.
            if let Some(new_title) = tab.browser.sync_title() {
                title_updates.push((tab.browser.id, new_title));
            }
            // Drain navigated URL changes (link clicks, redirects, etc.)
            // so the address bar stays in sync with the webview.
            if let Some(nav_url) = tab.browser.sync_nav_url() {
                url_updates.push((tab.browser.id, nav_url));
            }
        }

        // Sync webview title changes to the tile tree panes.
        for (wid, new_title) in title_updates {
            tiles::update_browser_view_title(&mut state.ui.tile_tree, wid, &new_title);
        }
        // Sync webview URL changes to the tile tree panes so
        // the docked address bar (which reads from the Pane's own url
        // field) stays in sync with the actual webview URL.
        for (wid, new_url) in url_updates {
            tiles::update_browser_view_url(&mut state.ui.tile_tree, wid, &new_url);
        }

        // Open new webview tabs for intercepted window.open() URLs.
        for url in new_window_urls {
            log::info!("Opening new webview tab from window.open(): {}", url);
            let (ui, mut ctx) = state.split_ui();
            ui.open_browser(&mut ctx, &url, event_loop);
        }

        // Poll pending terminal executions for marker detection.
        if !state.pending_terminal_execs.is_empty() {
            let sessions: Vec<_> = state.sessions.iter().collect();
            let mut completed = Vec::new();
            for (idx, pending) in state.pending_terminal_execs.iter().enumerate() {
                let elapsed_ms = pending.started_at.elapsed().as_millis() as u64;
                let timed_out = elapsed_ms >= pending.timeout_ms;

                // Look up the terminal by index.
                if let Some((_sid, session)) = sessions.get(pending.terminal_id) {
                    // Capture text from command start to current cursor.
                    let cur_row = session.state.abs_cursor_row();
                    let text = session
                        .state
                        .capture_text(pending.start_abs_row, cur_row + 1);
                    let lines: Vec<&str> = text.lines().collect();

                    // Find the LAST line whose trimmed content exactly matches
                    // the marker. This avoids false positives from the marker
                    // appearing inside the echoed command (e.g. `echo "MARKER"`).
                    let marker_line_idx = lines.iter().rposition(|l| l.trim() == pending.marker);

                    if let Some(marker_idx) = marker_line_idx {
                        // Calculate how many grid rows the command echo spans.
                        // The echo includes the shell prompt + the full command.
                        // We estimate the prompt at ~4 chars ("$ ") and use the
                        // terminal column width to compute wrapped lines.
                        let cols = session.grid_size.cols.max(1) as usize;
                        let cmd_echo_chars = 4 + pending.full_cmd.len(); // prompt + cmd
                        let echo_line_count = cmd_echo_chars.div_ceil(cols);
                        // Clamp: never skip past the marker line.
                        let skip = echo_line_count.min(marker_idx);

                        let output = lines[skip..marker_idx].join("\n").trim().to_string();

                        log::info!(
                            "TerminalExec: marker found at line {}/{}, cols={}, echo_lines={}, output_len={}, text_preview={:?}",
                            marker_idx,
                            lines.len(),
                            cols,
                            echo_line_count,
                            output.len(),
                            &text[..text.len().min(300)],
                        );

                        let truncated = output.len() > 50_000;
                        let output = if truncated {
                            output[..50_000].to_string()
                        } else {
                            output
                        };
                        completed.push((
                            idx,
                            serde_json::json!({
                                "exit_marker_found": true,
                                "output": output,
                                "truncated": truncated,
                                "elapsed_ms": elapsed_ms,
                            }),
                        ));
                    } else if timed_out {
                        // Timeout — return partial output.
                        // Skip the first line (command echo) as a best effort.
                        let output = lines
                            .iter()
                            .skip(1)
                            .copied()
                            .collect::<Vec<_>>()
                            .join("\n")
                            .trim()
                            .to_string();
                        log::warn!(
                            "TerminalExec: timed out after {}ms, lines={}, output_len={}, text_preview={:?}",
                            elapsed_ms,
                            lines.len(),
                            output.len(),
                            &text[..text.len().min(300)],
                        );
                        completed.push((
                            idx,
                            serde_json::json!({
                                "exit_marker_found": false,
                                "output": output,
                                "truncated": false,
                                "elapsed_ms": elapsed_ms,
                            }),
                        ));
                    }
                } else if timed_out {
                    completed.push((
                        idx,
                        serde_json::json!({
                            "exit_marker_found": false,
                            "output": "",
                            "error": format!("Terminal session {} not found", pending.terminal_id),
                            "elapsed_ms": elapsed_ms,
                        }),
                    ));
                }
            }
            // Resolve completed execs (iterate in reverse to preserve indices).
            for (idx, result) in completed.into_iter().rev() {
                let pending = state.pending_terminal_execs.remove(idx);
                if let Ok(mut map) = state.pending_results.lock()
                    && let Some(sender) = map.remove(&pending.marker)
                {
                    let _ = sender.send(result);
                }
            }
            // If there are still pending execs, schedule a timer to
            // keep polling without forcing unnecessary redraws.
            if !state.pending_terminal_execs.is_empty() {
                state
                    .scheduler
                    .schedule_at(Instant::now() + Duration::from_millis(100));
            }
        }

        // 5. Register timer deadlines with the scheduler.
        if let Some(blink) = state.next_cursor_blink {
            state.scheduler.schedule_at(blink);
        }

        // 6. Let the scheduler decide and apply: redraw + control flow.
        state
            .scheduler
            .apply(Instant::now(), &state.ui.frame.window, event_loop);
    }
}
