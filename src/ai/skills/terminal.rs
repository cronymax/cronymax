//! Terminal skills — split, mode toggle, send command, list terminals,
//! execute command with output capture, and read screen.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::{AppEvent, PendingResultMap};
use crate::ui::actions::UiAction;
use crate::ui::types::TerminalInfo;

/// Register all terminal skills into the registry.
pub fn register_terminal_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
    terminal_info: Arc<std::sync::Mutex<Vec<TerminalInfo>>>,
) {
    register_split_terminal(registry, proxy.clone());
    register_send_terminal_command(registry, proxy.clone());
    register_list_terminals(registry, terminal_info);
    register_terminal_execute_command(registry, proxy.clone(), pending_results.clone());
    register_terminal_read_screen(registry, proxy.clone(), pending_results.clone());
    register_reference_terminal_output(registry, proxy, pending_results);
}

fn register_split_terminal(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.terminal.split".into(),
        description: "Split the current terminal pane in a given direction.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "direction": {
                    "type": "string",
                    "enum": ["right", "down", "left"],
                    "description": "Direction to split: 'right' for horizontal, 'down' for vertical, 'left' for left split."
                }
            },
            "required": ["direction"]
        }),
        category: "terminal".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let direction = args["direction"].as_str().unwrap_or("right").to_string();

            let action = match direction.as_str() {
                "down" => UiAction::SplitDown,
                "left" => UiAction::SplitLeft,
                _ => UiAction::SplitRight,
            };

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action,
                result: json!({ "status": "split", "direction": direction }).to_string(),
            });

            Ok(json!({ "status": "split", "direction": direction }))
        })
    });

    registry.register(skill, handler);
}

fn register_send_terminal_command(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.terminal.send_command".into(),
        description: "Send a command to a terminal session's PTY as if the user typed it.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command text to send."
                },
                "session_id": {
                    "type": "integer",
                    "description": "Terminal session ID. Use 0 for the currently active session."
                }
            },
            "required": ["command"]
        }),
        category: "terminal".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let command = args["command"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?
                .to_string();
            let _session_id = args["session_id"].as_u64().unwrap_or(0) as u32;

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::ExecuteCommand(command.clone()),
                result: json!({ "status": "sent", "command": command }).to_string(),
            });

            Ok(json!({ "status": "sent", "command": command }))
        })
    });

    registry.register(skill, handler);
}

fn register_list_terminals(
    registry: &mut SkillRegistry,
    terminal_info: Arc<std::sync::Mutex<Vec<TerminalInfo>>>,
) {
    let skill = Skill {
        name: "cronymax.terminal.list".into(),
        description: "List all open terminal sessions with their IDs, titles, and modes.".into(),
        parameters_schema: json!({ "type": "object", "properties": {} }),
        category: "terminal".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let info = terminal_info.clone();
        Box::pin(async move {
            let terminals = info
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let items: Vec<Value> = terminals
                .iter()
                .map(|t| {
                    json!({
                        "session_id": t.session_id,
                        "title": t.title,
                        "pid": t.pid,
                        "cwd": t.cwd,
                        "running": t.running,
                    })
                })
                .collect();
            Ok(json!({
                "terminals": items,
                "count": items.len(),
            }))
        })
    });

    registry.register(skill, handler);
}

fn register_terminal_execute_command(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.terminal.execute".into(),
        description: "Execute a shell command in the app's terminal and capture its output. \
            The command runs in the selected terminal session (visible to the user). \
            Returns the captured stdout/stderr text."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "terminal_id": {
                    "type": "integer",
                    "description": "Terminal session index (0-based). Default: 0 (first terminal)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Maximum time to wait for output in milliseconds. Default: 30000"
                }
            },
            "required": ["command"]
        }),
        category: "terminal".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let command = args["command"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?
                .to_string();
            let terminal_id = args["terminal_id"].as_u64().unwrap_or(0) as usize;
            let timeout_ms = args["timeout_ms"].as_u64().unwrap_or(30_000);

            // Generate unique marker for output capture.
            let marker = format!("__CRONYMAX_DONE_{}__", uuid::Uuid::new_v4());

            // Create oneshot channel for receiving the result.
            let (tx, rx) = tokio::sync::oneshot::channel();

            // Register the sender in the pending map under the marker key.
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(marker.clone(), tx);
            }

            // Send the exec event to the main thread.
            let _ = proxy.send_event(AppEvent::TerminalExec {
                terminal_id,
                command: command.clone(),
                marker: marker.clone(),
                timeout_ms,
            });

            // Wait for the result (with timeout).
            match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms + 1000), rx)
                .await
            {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(_)) => Ok(json!({
                    "error": "Result channel closed unexpectedly",
                    "exit_marker_found": false,
                    "output": "",
                    "elapsed_ms": 0
                })),
                Err(_) => {
                    // Clean up the pending entry on timeout.
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&marker);
                    }
                    Ok(json!({
                        "error": "Timed out waiting for terminal output",
                        "exit_marker_found": false,
                        "output": "",
                        "elapsed_ms": timeout_ms
                    }))
                }
            }
        })
    });

    registry.register(skill, handler);
}

fn register_terminal_read_screen(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.terminal.read_screen".into(),
        description: "Read the current content visible on a terminal's screen. \
            Returns the text content of the terminal viewport or a specified line range \
            from the scrollback buffer."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "terminal_id": {
                    "type": "integer",
                    "description": "Terminal session index (0-based). Default: 0"
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line to read (0-based from top of scrollback). If omitted, reads the current viewport."
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read (inclusive). If omitted, reads to end of viewport."
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Maximum number of lines to return. Default: 100"
                }
            },
            "required": []
        }),
        category: "terminal".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let terminal_id = args["terminal_id"].as_u64().unwrap_or(0) as usize;
            let start_line = args["start_line"].as_i64().map(|v| v as i32);
            let end_line = args["end_line"].as_i64().map(|v| v as i32);
            let max_lines = args["max_lines"].as_u64().unwrap_or(100) as usize;

            let request_id = uuid::Uuid::new_v4().to_string();

            // Create oneshot channel.
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            // Send the read event to the main thread.
            let _ = proxy.send_event(AppEvent::ReadTerminalScreen {
                terminal_id,
                start_line,
                end_line,
                max_lines,
                request_id: request_id.clone(),
            });

            // Wait for the result.
            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(_)) => Ok(json!({
                    "error": "Result channel closed unexpectedly"
                })),
                Err(_) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({
                        "error": "Timed out reading terminal screen"
                    }))
                }
            }
        })
    });

    registry.register(skill, handler);
}

// ─── reference_terminal_output ────────────────────────────────────────────────

fn register_reference_terminal_output(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.terminal.reference_output".into(),
        description: "Read terminal output for a specific line range. The content can be \
            used as reference material for the conversation."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "terminal_id": { "type": "integer", "description": "Terminal session ID." },
                "start_line": { "type": "integer", "description": "Start line (absolute row, 0-based). Omit for beginning." },
                "end_line": { "type": "integer", "description": "End line (exclusive). Omit for end of buffer." }
            },
            "required": ["terminal_id"]
        }),
        category: "terminal".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let terminal_id = args["terminal_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'terminal_id'"))?
                as u32;
            let start_line = args["start_line"].as_i64().map(|v| v as i32);
            let end_line = args["end_line"].as_i64().map(|v| v as i32);
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending.lock().map_err(|e| anyhow::anyhow!("Lock: {}", e))?;
                map.insert(request_id.clone(), tx);
            }
            let _ = proxy.send_event(AppEvent::ReferenceTerminalOutput {
                terminal_id,
                start_line,
                end_line,
                request_id: request_id.clone(),
            });
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
                Ok(Ok(val)) => Ok(val),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({ "error": "Timeout reading terminal output" }))
                }
            }
        })
    });

    registry.register(skill, handler);
}
