//! Tab skills — create, close, switch, list, channel lifecycle, popout/dock, rename, pin.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::AppEvent;
use crate::ui::actions::UiAction;
use crate::ui::types::TabInfo;

/// Register all tab skills into the registry.
pub fn register_tab_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    tab_info: Arc<std::sync::Mutex<Vec<TabInfo>>>,
) {
    register_create_tab(registry, proxy.clone());
    register_close_tab(registry, proxy.clone());
    register_switch_tab(registry, proxy.clone());
    register_list_tabs(registry, tab_info);
    register_open_channel_tab(registry, proxy.clone());
    register_close_channel(registry, proxy.clone());
    register_popout_webview(registry, proxy.clone());
    register_hide_webview_popout(registry, proxy.clone());
    register_rename_tab(registry, proxy.clone());
    register_pin_tab(registry, proxy.clone());
}

fn register_create_tab(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.create".into(),
        description: "Create a new tab. Modes: 'chat' (prompt + PTY), 'terminal' (raw PTY), or 'channel' (channel tab).".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["chat", "terminal", "channel"],
                    "description": "Tab mode. Default: 'chat'."
                },
                "channel_id": {
                    "type": "string",
                    "description": "Required when mode is 'channel'. The channel ID to open."
                },
                "channel_name": {
                    "type": "string",
                    "description": "Display name for the channel tab."
                }
            }
        }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let mode = args["mode"].as_str().unwrap_or("chat");
            let action = match mode {
                "terminal" => UiAction::NewTerminal,
                "channel" => {
                    let channel_id = args["channel_id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'channel_id' for channel mode"))?
                        .to_string();
                    let channel_name = args["channel_name"]
                        .as_str()
                        .unwrap_or(&channel_id)
                        .to_string();
                    UiAction::OpenChannelTab {
                        channel_id,
                        channel_name,
                    }
                }
                _ => UiAction::NewChat,
            };

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action,
                result: json!({ "status": "created", "mode": mode }).to_string(),
            });

            Ok(json!({ "status": "created", "mode": mode }))
        })
    });

    registry.register(skill, handler);
}

fn register_close_tab(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.close".into(),
        description: "Close a terminal tab by session ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "integer",
                    "description": "Terminal session ID to close."
                }
            },
            "required": ["session_id"]
        }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' argument"))?
                as u32;

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::CloseTab(session_id),
                result: json!({ "status": "closed", "session_id": session_id }).to_string(),
            });

            Ok(json!({ "status": "closed", "session_id": session_id }))
        })
    });

    registry.register(skill, handler);
}

fn register_switch_tab(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.switch".into(),
        description: "Switch the active terminal tab by index or session ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "integer",
                    "description": "Terminal session ID to switch to."
                },
                "index": {
                    "type": "integer",
                    "description": "Zero-based tab index to switch to."
                }
            }
        }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let index = args["index"].as_u64().unwrap_or(0) as usize;

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::SwitchTab(index),
                result: json!({ "status": "switched", "index": index }).to_string(),
            });

            Ok(json!({ "status": "switched", "index": index }))
        })
    });

    registry.register(skill, handler);
}

fn register_list_tabs(registry: &mut SkillRegistry, tab_info: Arc<std::sync::Mutex<Vec<TabInfo>>>) {
    let skill = Skill {
        name: "cronymax.tab.list".into(),
        description: "List all open tabs including chat, terminal, browser, and channel tabs."
            .into(),
        parameters_schema: json!({ "type": "object", "properties": {} }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let info = tab_info.clone();
        Box::pin(async move {
            let tabs = info
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let items: Vec<Value> = tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let (kind, extra) = match t {
                        TabInfo::Chat { session_id, title } => {
                            ("chat", json!({ "session_id": session_id, "title": title }))
                        }
                        TabInfo::Terminal { session_id, title } => (
                            "terminal",
                            json!({ "session_id": session_id, "title": title }),
                        ),
                        TabInfo::BrowserView {
                            webview_id,
                            title,
                            url,
                            ..
                        } => (
                            "browser",
                            json!({ "webview_id": webview_id, "title": title, "url": url }),
                        ),
                        TabInfo::Channel {
                            channel_id,
                            channel_name,
                        } => (
                            "channel",
                            json!({ "channel_id": channel_id, "channel_name": channel_name }),
                        ),
                    };
                    let mut obj = extra;
                    obj["index"] = json!(i);
                    obj["kind"] = json!(kind);
                    obj
                })
                .collect();
            Ok(json!({
                "tabs": items,
                "count": items.len(),
            }))
        })
    });

    registry.register(skill, handler);
}

// ─── open_channel_tab ─────────────────────────────────────────────────────────

fn register_open_channel_tab(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.open_channel".into(),
        description: "Open a channel tab by channel ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel ID to open." },
                "channel_name": { "type": "string", "description": "Display name for the channel tab." }
            },
            "required": ["channel_id"]
        }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let channel_id = args["channel_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'channel_id'"))?
                .to_string();
            let channel_name = args["channel_name"]
                .as_str()
                .unwrap_or(&channel_id)
                .to_string();
            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::OpenChannelTab {
                    channel_id: channel_id.clone(),
                    channel_name: channel_name.clone(),
                },
                result: json!({ "status": "opened", "channel_id": channel_id }).to_string(),
            });
            Ok(
                json!({ "status": "opened", "channel_id": channel_id, "channel_name": channel_name }),
            )
        })
    });

    registry.register(skill, handler);
}

// ─── close_channel ────────────────────────────────────────────────────────────

fn register_close_channel(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.close_channel".into(),
        description: "Close a channel tab by channel ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel ID to close." }
            },
            "required": ["channel_id"]
        }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let channel_id = args["channel_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'channel_id'"))?
                .to_string();
            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::CloseChannel(channel_id.clone()),
                result: json!({ "status": "closed", "channel_id": channel_id }).to_string(),
            });
            Ok(json!({ "status": "closed", "channel_id": channel_id }))
        })
    });

    registry.register(skill, handler);
}

// ─── popout_webview ───────────────────────────────────────────────────────────

fn register_popout_webview(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.popout_webview".into(),
        description: "Pop out the active overlay webview into an independent child window.".into(),
        parameters_schema: json!({ "type": "object", "properties": {} }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::PopOutOverlay,
                result: json!({ "status": "popped_out" }).to_string(),
            });
            Ok(json!({ "status": "popped_out" }))
        })
    });

    registry.register(skill, handler);
}

// ─── hide_webview_popout ──────────────────────────────────────────────────────

fn register_hide_webview_popout(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.hide_webview_popout".into(),
        description: "Dock the active overlay webview back into the main window as a split.".into(),
        parameters_schema: json!({ "type": "object", "properties": {} }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::DockWebview,
                result: json!({ "status": "docked" }).to_string(),
            });
            Ok(json!({ "status": "docked" }))
        })
    });

    registry.register(skill, handler);
}

// ─── rename_tab ───────────────────────────────────────────────────────────────

fn register_rename_tab(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.rename".into(),
        description: "Rename a tab by session ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Session ID of the tab to rename." },
                "title": { "type": "string", "description": "New tab title." }
            },
            "required": ["session_id", "title"]
        }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let title = args["title"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'title'"))?
                .to_string();
            let _ = proxy.send_event(AppEvent::RenameTab {
                session_id,
                title: title.clone(),
            });
            Ok(json!({ "status": "renamed", "session_id": session_id, "title": title }))
        })
    });

    registry.register(skill, handler);
}

// ─── pin_tab ──────────────────────────────────────────────────────────────────

fn register_pin_tab(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.tab.pin".into(),
        description: "Pin or unpin a tab by session ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Session ID of the tab to pin/unpin." },
                "pinned": { "type": "boolean", "description": "True to pin, false to unpin." }
            },
            "required": ["session_id", "pinned"]
        }),
        category: "tab".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let pinned = args["pinned"].as_bool().unwrap_or(true);
            let action = if pinned {
                UiAction::PinTab(session_id)
            } else {
                UiAction::UnpinTab(session_id)
            };
            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action,
                result: json!({ "status": if pinned { "pinned" } else { "unpinned" }, "session_id": session_id }).to_string(),
            });
            Ok(
                json!({ "status": if pinned { "pinned" } else { "unpinned" }, "session_id": session_id }),
            )
        })
    });

    registry.register(skill, handler);
}
