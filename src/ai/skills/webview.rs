//! Webview skills — open, navigate, close, and list webview tabs.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::AppEvent;
use crate::ui::actions::UiAction;
use crate::ui::types::BrowserViewInfo;

/// Register all webview skills into the registry.
pub fn register_webview_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    webview_info: Arc<std::sync::Mutex<Vec<BrowserViewInfo>>>,
) {
    register_open_webview(registry, proxy.clone());
    register_navigate_webview(registry, proxy.clone());
    register_close_webview(registry, proxy.clone());
    register_list_webviews(registry, webview_info);
}

fn register_open_webview(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.webview.open".into(),
        description: "Open a URL in a new browser tab within the terminal application.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to open."
                },
                "title": {
                    "type": "string",
                    "description": "Optional tab title. Defaults to the URL domain."
                }
            },
            "required": ["url"]
        }),
        category: "webview".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let url = args["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' argument"))?
                .to_string();
            let _title = args["title"].as_str().unwrap_or_default().to_string();

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::OpenWebviewTab(url.clone()),
                result: json!({
                    "status": "opened",
                    "url": url,
                })
                .to_string(),
            });

            Ok(
                json!({ "status": "opened", "url": url, "webview_id": 0, "note": "Use webview_id 0 to interact with the active webview via browser skills." }),
            )
        })
    });

    registry.register(skill, handler);
}

fn register_navigate_webview(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.webview.navigate".into(),
        description: "Navigate an existing browser tab to a new URL.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to navigate to."
                },
                "webview_id": {
                    "type": "integer",
                    "description": "ID of the webview to navigate. Use 0 for the currently active webview."
                }
            },
            "required": ["url"]
        }),
        category: "webview".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let url = args["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' argument"))?
                .to_string();
            let webview_id = args["webview_id"].as_u64().unwrap_or(0) as u32;

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::NavigateWebview(url.clone(), webview_id),
                result: json!({
                    "status": "navigated",
                    "webview_id": webview_id,
                    "url": url,
                })
                .to_string(),
            });

            Ok(json!({
                "status": "navigated",
                "webview_id": webview_id,
                "url": url,
            }))
        })
    });

    registry.register(skill, handler);
}

fn register_close_webview(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.webview.close".into(),
        description: "Close a browser tab.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": {
                    "type": "integer",
                    "description": "ID of the webview to close. Use 0 for the currently active webview."
                }
            },
            "required": ["webview_id"]
        }),
        category: "webview".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"].as_u64().unwrap_or(0) as u32;

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action: UiAction::CloseWebviewTab(webview_id),
                result: json!({
                    "status": "closed",
                    "webview_id": webview_id,
                })
                .to_string(),
            });

            Ok(json!({ "status": "closed", "webview_id": webview_id }))
        })
    });

    registry.register(skill, handler);
}

fn register_list_webviews(
    registry: &mut SkillRegistry,
    webview_info: Arc<std::sync::Mutex<Vec<BrowserViewInfo>>>,
) {
    let skill = Skill {
        name: "cronymax.webview.list".into(),
        description: "List all open browser tabs with their IDs, titles, and URLs.".into(),
        parameters_schema: json!({ "type": "object", "properties": {} }),
        category: "webview".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let info = webview_info.clone();
        Box::pin(async move {
            let tabs = info.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            let webviews: Vec<Value> = tabs
                .iter()
                .map(|t| {
                    json!({
                        "id": t.webview_id,
                        "title": t.title,
                        "url": t.url,
                    })
                })
                .collect();
            Ok(json!({ "webviews": webviews }))
        })
    });

    registry.register(skill, handler);
}
