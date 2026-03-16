//! Window and user event handlers extracted from app/mod.rs

use crate::app::*;

pub(in crate::app) fn handle_window_event(
    app: &mut App,
    event_loop: &ActiveEventLoop,
    window_id: WindowId,
    event: WindowEvent,
) {
    let state = match app.state.as_mut() {
        Some(s) => s,
        None => return,
    };

    // Route events from child/owned windows (Windows overlay panels).
    // On macOS, child panels are NSPanels without winit WindowIds — events
    // are captured via the NSEvent local monitor installed in ModalPanel.
    // On Windows, child panels ARE winit windows, so events arrive here.
    #[cfg(target_os = "windows")]
    if window_id != state.window.id() {
        // Check if this event belongs to any overlay child panel.
        for wt in &state.webview_tabs {
            if wt.mode == BrowserViewMode::Overlay
                && wt.manager.visible
                && wt
                    .manager
                    .overlay
                    .as_ref()
                    .and_then(|o| o.panel.window_id())
                    == Some(window_id)
            {
                // Convert the winit WindowEvent to egui events and push
                // into the overlay's event buffer for render().
                let scale = state.window.scale_factor() as f32;
                let egui_events = winit_event_to_egui(&event, scale);
                if !egui_events.is_empty()
                    && let Some(overlay) = &wt.manager.overlay
                {
                    // Update the persistent last-cursor-pos on PointerMoved
                    // events so PointerButton can use it even after the
                    // event buffer has been drained by a render frame.
                    for ev in &egui_events {
                        if let egui::Event::PointerMoved(pos) = ev
                            && let Ok(mut lcp) = overlay.panel.last_cursor_pos.lock()
                        {
                            *lcp = *pos;
                        }
                    }
                    if let Ok(mut buf) = overlay.panel.event_buffer.lock() {
                        let last_pos = overlay
                            .panel
                            .last_cursor_pos
                            .lock()
                            .map(|p| *p)
                            .unwrap_or(egui::Pos2::ZERO);
                        let fixed: Vec<egui::Event> = egui_events
                            .into_iter()
                            .map(|ev| {
                                if let egui::Event::PointerButton {
                                    pos,
                                    button,
                                    pressed,
                                    modifiers,
                                } = ev
                                {
                                    if pos == egui::Pos2::ZERO {
                                        egui::Event::PointerButton {
                                            pos: last_pos,
                                            button,
                                            pressed,
                                            modifiers,
                                        }
                                    } else {
                                        ev
                                    }
                                } else {
                                    ev
                                }
                            })
                            .collect();
                        buf.extend(fixed);
                    }
                }
                // Bring this overlay to front on focus or mouse-down (T023).
                let should_activate = matches!(
                    event,
                    WindowEvent::Focused(true)
                        | WindowEvent::MouseInput {
                            state: winit::event::ElementState::Pressed,
                            ..
                        }
                );
                if should_activate {
                    let wid = wt.id;
                    state.webview_manager.bring_to_front(wid);
                    if let Some(idx) = state.webview_tabs.iter().position(|wt| wt.id == wid) {
                        state.active_webview = idx;
                        state.ui_state.active_webview = Some(idx);
                    }
                }
                return;
            }
        }
        return;
    }

    // On non-Windows, skip events from popover child/owned windows
    // (macOS NSPanel).  Only process events for the main window.
    #[cfg(not(target_os = "windows"))]
    if window_id != state.window.id() {
        return;
    }

    match event {
        WindowEvent::CloseRequested => {
            log::info!("Window close requested");

            // ── On-exit session persistence (T035) ───────────────────
            // Synchronously save all chat sessions, layout, and command
            // history before the process exits.
            {
                let mgr = state.profile_manager.lock().unwrap();
                let profile_dir = mgr
                    .active()
                    .map(|p| mgr.profile_dir(&p.id))
                    .unwrap_or_else(|| mgr.profile_dir("default"));
                drop(mgr);

                // Save each chat session.
                for chat in state.session_chats.values() {
                    if let Some(ref pid) = chat.persistent_id {
                        let record = crate::app::session_persist::chat_to_record(pid, chat);
                        if let Err(e) = crate::app::session_persist::save_session_file(
                            pid,
                            &record,
                            &profile_dir,
                        ) {
                            log::warn!("Exit-save session {}: {}", pid, e);
                        }
                    }
                }

                // Save layout snapshot.
                let snapshot = crate::app::session_persist::serialize_layout(
                    &state.tile_tree,
                    &state.session_chats,
                );
                if let Err(e) = crate::app::session_persist::save_layout(&snapshot, &profile_dir) {
                    log::warn!("Exit-save layout: {}", e);
                }

                // Save command history (merge all prompt editors).
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let entries: Vec<crate::app::session_persist::CommandHistoryEntry> = state
                    .prompt_editors
                    .values()
                    .flat_map(|pe| pe.history.iter())
                    .map(|cmd| crate::app::session_persist::CommandHistoryEntry {
                        command: cmd.clone(),
                        timestamp: now,
                    })
                    .collect();
                if !entries.is_empty()
                    && let Err(e) =
                        crate::app::session_persist::save_command_history(&entries, &profile_dir)
                {
                    log::warn!("Exit-save command history: {}", e);
                }
            }

            // Explicitly drop all child panels and GPU surfaces to prevent
            // orphaned child windows lingering after the main window closes.
            for tab in &mut state.webview_tabs {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                {
                    tab.manager.overlay = None;
                }
            }
            event_loop.exit();
        }

        // Forward all events to egui first, then handle normally.
        ref event @ WindowEvent::ModifiersChanged(_)
        | ref event @ WindowEvent::KeyboardInput { .. }
        | ref event @ WindowEvent::Ime(_)
        | ref event @ WindowEvent::CursorMoved { .. }
        | ref event @ WindowEvent::MouseInput { .. }
        | ref event @ WindowEvent::MouseWheel { .. }
        | ref event @ WindowEvent::Touch(_) => {
            // Always track modifiers regardless of egui consumption.
            if let WindowEvent::ModifiersChanged(new_modifiers) = event {
                state.modifiers = new_modifiers.state();
            }

            // Always track mouse position.
            if let WindowEvent::CursorMoved { position, .. } = event {
                state.mouse_x = position.x as f32;
                state.mouse_y = position.y as f32;

                // ── Cmd/Ctrl+hover link detection ────────────────────
                let super_held = if cfg!(target_os = "macos") {
                    state.modifiers.super_key()
                } else {
                    state.modifiers.control_key()
                };
                if super_held {
                    if let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                        && let Some(session) = state.sessions.get(&sid)
                    {
                        let vp = &state.viewport;
                        let cell = &state.renderer.cell_size;
                        let px = state.mouse_x - vp.x;
                        let py = state.mouse_y - vp.y;
                        if px >= 0.0 && py >= 0.0 {
                            let col = (px / cell.width) as usize;
                            let row = (py / cell.height) as usize;
                            let (grid_cols, _) = vp.grid_dimensions(cell);
                            let link = crate::renderer::terminal::links::link_at(
                                session.state.term(),
                                col,
                                row,
                                grid_cols as usize,
                            );
                            if link.is_some() {
                                state.window.set_cursor(winit::window::CursorIcon::Pointer);
                            } else {
                                state.window.set_cursor(winit::window::CursorIcon::Default);
                            }
                            state.hovered_link = link;
                            state.scheduler.mark_dirty();
                        }
                    }
                } else if state.hovered_link.is_some() {
                    state.hovered_link = None;
                    state.window.set_cursor(winit::window::CursorIcon::Default);
                    state.scheduler.mark_dirty();
                }
            }

            // Determine active tab mode EARLY so we can route correctly.
            // Look up active mode by session_id (split panes may
            // not be in terminal_tabs).
            // Use focused_terminal_session (set by pane click detection)
            // so keyboard routes to the correct split pane.  Fall back
            // to active_terminal_session when no pane has been clicked yet.
            let focused_sid = state
                .ui_state
                .focused_terminal_session
                .filter(|sid| state.sessions.contains_key(sid))
                .or_else(|| tiles::active_terminal_session(&state.tile_tree));

            // ── Check app-level keybindings BEFORE egui gets the event ──
            // This ensures Ctrl+Shift+P, etc.
            // always work even when an egui TextEdit has focus.
            // When an egui TextEdit has focus, let Copy/Paste flow to egui
            // so the user can copy/paste in the prompt editor and address bar.
            if let WindowEvent::KeyboardInput {
                event: key_ev,
                is_synthetic: false,
                ..
            } = event
                && key_ev.state == winit::event::ElementState::Pressed
                && let Some(action) = match_keybinding(key_ev, &state.modifiers)
            {
                // If egui wants keyboard input (e.g. TextEdit focused),
                // let Copy/Paste pass through to egui instead.
                let egui_wants = state.egui.wants_keyboard_input();
                let is_clipboard_action = matches!(action, Action::Copy | Action::Paste);
                if !(egui_wants && is_clipboard_action) {
                    handle_action(state, action);
                    return;
                }
            }

            // ── Track IME composition state ─────────────────────────
            match event {
                WindowEvent::Ime(winit::event::Ime::Enabled) => {
                    state.ime_enabled = true;
                }
                WindowEvent::Ime(winit::event::Ime::Disabled) => {
                    state.ime_enabled = false;
                    state.ime_composing = false;
                }
                WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                    state.ime_composing = !text.is_empty();
                }
                WindowEvent::Ime(winit::event::Ime::Commit(_)) => {
                    state.ime_composing = false;
                }
                _ => {}
            }

            // ── In Terminal mode: forward keyboard to PTY, skip egui ────
            // egui still gets mouse/cursor events for the tab bar etc.
            let is_terminal = focused_sid
                .and_then(|sid| state.prompt_editors.get(&sid))
                .is_some_and(|pe| !pe.visible);
            if is_terminal {
                // Forward IME committed text to PTY.
                if let WindowEvent::Ime(winit::event::Ime::Commit(text)) = event {
                    if let Some(sid) = focused_sid
                        && let Some(session) = state.sessions.get_mut(&sid)
                    {
                        session.write_to_pty(text.as_bytes());
                    }
                    state.scheduler.mark_dirty();
                    return;
                }
                // Let IME preedit/enabled/disabled events pass through to egui.
                if matches!(event, WindowEvent::Ime(..)) {
                    state.egui.on_window_event(&state.window, event);
                    state.scheduler.mark_dirty();
                    return;
                }
                if let WindowEvent::KeyboardInput {
                    event: key_ev,
                    is_synthetic: false,
                    ..
                } = event
                {
                    if key_ev.state == winit::event::ElementState::Pressed {
                        // Command mode input in Terminal mode
                        if state.colon_buf.is_some() {
                            match &key_ev.logical_key {
                                winit::keyboard::Key::Character(c) => {
                                    state.colon_buf.as_mut().unwrap().push_str(c.as_str());
                                }
                                winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) => {
                                    state.colon_buf.as_mut().unwrap().push(' ');
                                }
                                winit::keyboard::Key::Named(
                                    winit::keyboard::NamedKey::Backspace,
                                ) => {
                                    let buf = state.colon_buf.as_mut().unwrap();
                                    if buf.is_empty() {
                                        state.colon_buf = None;
                                        log::info!("Command mode cancelled");
                                    } else {
                                        buf.pop();
                                    }
                                }
                                winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                                    let cmd = state.colon_buf.take().unwrap();
                                    handle_colon_command(state, &cmd, event_loop);
                                }
                                winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                                    state.colon_buf = None;
                                    log::info!("Command mode cancelled");
                                }
                                _ => {}
                            }
                            state.scheduler.mark_dirty();
                            return;
                        }

                        // Forward to PTY
                        if let Some(bytes) = input::encode_key(key_ev, &state.modifiers)
                            && let Some(sid) = focused_sid
                            && let Some(session) = state.sessions.get_mut(&sid)
                        {
                            session.write_to_pty(&bytes);
                        }
                    }
                    state.scheduler.mark_dirty();
                    return;
                }
            }

            // ── Forward event to egui ────────────────────────────────
            // During IME composition, suppress KeyboardInput events from
            // reaching egui — on macOS, winit fires raw key events (e.g.
            // 'n','i','h','a','o') alongside Preedit.  If egui sees them it
            // inserts garbled Latin characters into the TextEdit.  Only
            // Ime events (Preedit/Commit/Enabled/Disabled) should pass.
            // Suppress KeyboardInput events from reaching egui during
            // IME composition.  Also suppress character-key presses when
            // the IME input method is enabled to catch the very first
            // keystroke that arrives *before* the Preedit event.
            let suppress_for_ime = if state.ime_composing {
                matches!(event, WindowEvent::KeyboardInput { .. })
            } else if state.ime_enabled {
                matches!(
                    event,
                    WindowEvent::KeyboardInput {
                        event: winit::event::KeyEvent {
                            logical_key: winit::keyboard::Key::Character(_),
                            state: winit::event::ElementState::Pressed,
                            ..
                        },
                        ..
                    }
                )
            } else {
                false
            };
            let needs_redraw = if suppress_for_ime {
                // Don't give this event to egui at all.
                false
            } else {
                state.egui.on_window_event(&state.window, event)
            };

            // In Editor mode: let egui handle keyboard when it has focus
            // (TextEdit captures typing, Enter, etc.)
            // Only mark dirty when egui actually needs a visual update;
            // wants_keyboard_input() alone just gates event routing.
            if needs_redraw || state.egui.wants_keyboard_input() {
                if needs_redraw {
                    state.scheduler.mark_dirty();
                }
                return;
            }

            // ── Keyboard events not consumed by egui (Editor mode, no widget focused) ──
            if let WindowEvent::KeyboardInput {
                event: key_ev,
                is_synthetic: false,
                ..
            } = event
            {
                if key_ev.state != winit::event::ElementState::Pressed {
                    return;
                }

                // ── Command mode input ───────────────────────────────
                if state.colon_buf.is_some() {
                    match &key_ev.logical_key {
                        winit::keyboard::Key::Character(c) => {
                            state.colon_buf.as_mut().unwrap().push_str(c.as_str());
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) => {
                            state.colon_buf.as_mut().unwrap().push(' ');
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                            let buf = state.colon_buf.as_mut().unwrap();
                            if buf.is_empty() {
                                state.colon_buf = None;
                                log::info!("Command mode cancelled");
                            } else {
                                buf.pop();
                            }
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                            let cmd = state.colon_buf.take().unwrap();
                            handle_colon_command(state, &cmd, event_loop);
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                            state.colon_buf = None;
                            log::info!("Command mode cancelled");
                        }
                        _ => {}
                    }
                    state.scheduler.mark_dirty();
                    return;
                }

                // ── Encode keyboard input and send to active PTY ─────
                // Forward unconsumed keystrokes to the PTY regardless of
                // mode.  In Chat mode egui captures typing when its
                // TextEdit has focus (returned earlier above).  If we
                // reach here, no egui widget consumed the event, so the
                // PTY should receive it.
                if !state.ime_composing
                    && let Some(bytes) = input::encode_key(key_ev, &state.modifiers)
                    && let Some(sid) = focused_sid
                    && let Some(session) = state.sessions.get_mut(&sid)
                {
                    session.write_to_pty(&bytes);
                }
            }

            // ── Handle mouse clicks not consumed by egui ─────────────
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } = event
            {
                // ── Cmd/Ctrl+click: open hovered link ────────────────
                let super_held = if cfg!(target_os = "macos") {
                    state.modifiers.super_key()
                } else {
                    state.modifiers.control_key()
                };
                if super_held
                    && let Some(sid) = tiles::active_terminal_session(&state.tile_tree)
                    && let Some(session) = state.sessions.get(&sid)
                {
                    let vp = &state.viewport;
                    let cell = &state.renderer.cell_size;
                    let px = state.mouse_x - vp.x;
                    let py = state.mouse_y - vp.y;
                    if px >= 0.0 && py >= 0.0 {
                        let col = (px / cell.width) as usize;
                        let row = (py / cell.height) as usize;
                        let (grid_cols, _) = vp.grid_dimensions(cell);
                        if let Some(link) = crate::renderer::terminal::links::link_at(
                            session.state.term(),
                            col,
                            row,
                            grid_cols as usize,
                        ) {
                            let url = if link.is_path {
                                let resolved = crate::renderer::terminal::links::resolve_path(&link.url);
                                format!("file://{}", resolved)
                            } else {
                                link.url.clone()
                            };
                            log::info!("Opening link: {}", url);
                            open_webview(state, &url, event_loop);
                            return; // consume the click
                        }
                    }
                }
                handle_mouse_click(state);
            }
        }

        WindowEvent::Resized(new_size) => {
            super::misc::handle_resize(state, new_size);
        }

        WindowEvent::Focused(focused) => {
            super::misc::handle_focus(state, focused);
        }

        WindowEvent::ThemeChanged(_) => {
            super::misc::handle_theme_changed(state);
        }

        WindowEvent::ScaleFactorChanged { .. } => {
            super::misc::handle_scale_change(state);
        }

        WindowEvent::RedrawRequested => {
            crate::app::draw::handle_redraw(state, event_loop);
        }

        _ => {}
    }
}
