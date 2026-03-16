//! Onboarding skills — agent-driven channel setup wizard.
//!
//! These skills allow the AI agent to guide users through Feishu/Lark channel
//! configuration conversationally (status, start, step, credentials, test).
//! Category: "channels"
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::{AppEvent, PendingResultMap};
use crate::ui::actions::UiAction;

/// Shared onboarding wizard state visible to skill handlers.
pub type OnboardingState = Arc<std::sync::Mutex<Option<OnboardingWizardState>>>;

/// Wizard state for multi-step channel onboarding.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct OnboardingWizardState {
    pub active: bool,
    pub channel_type: String,
    pub current_step: u32,
    pub total_steps: u32,
    pub completed_steps: Vec<String>,
    pub errors: Vec<String>,
    /// When true, the wizard is embedded inside the internal browser view.
    pub guided_mode: bool,
    /// Target channel instance ID for multi-instance support.
    pub target_instance_id: Option<String>,
}

/// Register all onboarding skills into the registry.
pub fn register_onboarding_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
    onboarding_state: OnboardingState,
) {
    register_onboard_channel_status(registry, onboarding_state.clone());
    register_onboard_channel_start(registry, proxy.clone(), pending_results.clone());
    register_onboard_channel_step(registry, proxy.clone(), pending_results.clone());
    register_onboard_lark_open_console(registry, proxy.clone());
    register_onboard_lark_store_credentials(registry, proxy.clone(), pending_results.clone());
    register_onboard_lark_test_connection(registry, proxy.clone(), pending_results.clone());
    register_onboard_finalize_lark_onboard(registry, proxy, pending_results);
}

// ─── onboard_channel_status ──────────────────────────────────────────────────

fn register_onboard_channel_status(
    registry: &mut SkillRegistry,
    onboarding_state: OnboardingState,
) {
    let skill = Skill {
        name: "cronymax.channels.status".into(),
        description: "Check the current status of the channel onboarding wizard. \
            Returns whether a wizard is active, current step, and any errors."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {}
        }),
        category: "channels".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let state = onboarding_state.clone();
        Box::pin(async move {
            let guard = state
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            match guard.as_ref() {
                Some(s) => Ok(serde_json::to_value(s)?),
                None => Ok(json!({
                    "active": false,
                    "channel_type": null,
                    "current_step": 0,
                    "total_steps": 0,
                    "completed_steps": [],
                    "errors": [],
                    "guided_mode": false,
                    "target_instance_id": null,
                })),
            }
        })
    });

    registry.register(skill, handler);
}

// ─── onboard_channel_start ───────────────────────────────────────────────────

fn register_onboard_channel_start(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.channels.start".into(),
        description: "Start the channel onboarding wizard for a specific channel type. \
            Returns information about the first step."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "channel_type": {
                    "type": "string",
                    "enum": ["lark"],
                    "description": "Channel type to onboard (currently only 'lark' is supported)"
                },
                "instance_id": {
                    "type": "string",
                    "description": "Optional instance ID for multi-instance channel support. If omitted, a default ID is generated."
                }
            },
            "required": ["channel_type"]
        }),
        category: "channels".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let channel_type = args["channel_type"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'channel_type' argument"))?
                .to_string();
            let instance_id = args["instance_id"].as_str().map(|s| s.to_string());

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::OnboardingStart {
                channel_type,
                instance_id,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({ "error": "Timed out" }))
                }
            }
        })
    });

    registry.register(skill, handler);
}

// ─── onboard_channel_step ────────────────────────────────────────────────────

fn register_onboard_channel_step(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.channels.step".into(),
        description: "Advance or interact with the current onboarding wizard step. \
            Actions: 'next' to advance, 'skip' to skip current step, 'retry' to retry."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["next", "skip", "retry"],
                    "description": "Action to take on the current step"
                }
            },
            "required": ["action"]
        }),
        category: "channels".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let action = args["action"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' argument"))?
                .to_string();

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::OnboardingAdvanceStep {
                action,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({ "error": "Timed out" }))
                }
            }
        })
    });

    registry.register(skill, handler);
}

// ─── onboard_lark_open_console ───────────────────────────────────────────────

fn register_onboard_lark_open_console(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
) {
    let skill = Skill {
        name: "cronymax.channels.lark_open_console".into(),
        description: "Open the Lark Developer Console in the internal browser overlay. Optionally trigger the guided bot/permission automation for a specific app.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "app_id": {
                    "type": "string",
                    "description": "Optional Lark App ID. Required when automate_permissions is true so the overlay can target the selected app."
                },
                "automate_permissions": {
                    "type": "boolean",
                    "description": "If true, open the overlay and run the best-effort bot + batch-import permissions automation.",
                    "default": false
                }
            }
        }),
        category: "channels".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let automate_permissions = args["automate_permissions"].as_bool().unwrap_or(false);
            let app_id = args["app_id"].as_str().unwrap_or_default().to_string();
            if automate_permissions && app_id.trim().is_empty() {
                return Ok(json!({
                    "error": "app_id is required when automate_permissions is true"
                }));
            }

            let action = if automate_permissions {
                UiAction::OnboardingAutomateLarkSetup {
                    app_id: app_id.clone(),
                }
            } else {
                UiAction::OpenWebviewTab("https://open.feishu.cn/app".into())
            };
            let status = if automate_permissions {
                "opened and automation requested"
            } else {
                "opened"
            };
            let note = if automate_permissions {
                "Select or create the app in the overlay if needed, then review the bot and permissions pages before storing credentials."
            } else {
                "Create or select the target app in the overlay, then call this skill again with automate_permissions=true to run the bot and permissions automation."
            };
            let result = json!({
                "status": status,
                "url": "https://open.feishu.cn/app",
                "app_id": if app_id.is_empty() { Value::Null } else { Value::String(app_id.clone()) },
                "recommended_permissions": [
                    "im:message",
                    "im:chat",
                    "im:message.group_at_msg"
                ],
                "next_steps": [
                    "Create or select the Lark app in the overlay browser.",
                    "Enable the bot capability and import the recommended permissions.",
                    "Use onboard_lark_store_credentials to store the App ID and App Secret securely.",
                    "Use onboard_lark_test_connection to verify the final channel config."
                ],
                "note": note
            });

            let _ = proxy.send_event(AppEvent::SkillUiAction {
                session_id: 0,
                tool_call_id: String::new(),
                action,
                result: result.to_string(),
            });

            Ok(result)
        })
    });

    registry.register(skill, handler);
}

// ─── onboard_lark_store_credentials ──────────────────────────────────────────

fn register_onboard_lark_store_credentials(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.channels.lark_store_credentials".into(),
        description: "Store Lark app credentials (app_id and app_secret) securely. \
            These are needed to connect to the Feishu/Lark Open Platform."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "app_id": {
                    "type": "string",
                    "description": "Lark App ID (starts with 'cli_')"
                },
                "app_secret": {
                    "type": "string",
                    "description": "Lark App Secret"
                }
            },
            "required": ["app_id", "app_secret"]
        }),
        category: "channels".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let app_id = args["app_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'app_id' argument"))?
                .to_string();
            let app_secret = args["app_secret"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'app_secret' argument"))?
                .to_string();

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::OnboardingStoreCredentials {
                app_id,
                app_secret,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({ "error": "Timed out" }))
                }
            }
        })
    });

    registry.register(skill, handler);
}

// ─── onboard_lark_test_connection ────────────────────────────────────────────

fn register_onboard_lark_test_connection(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.channels.lark_test_connection".into(),
        description: "Test the Lark channel connection using stored credentials. \
            Validates the app_id/app_secret, checks bot permissions, and tests \
            WebSocket reachability. Returns detailed diagnostics."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {}
        }),
        category: "channels".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::OnboardingTestConnection {
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({ "error": "Timed out after 15s" }))
                }
            }
        })
    });

    registry.register(skill, handler);
}

// ─── onboard_finalize_lark_onboard ───────────────────────────────────────────────────

fn register_onboard_finalize_lark_onboard(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.channels.finalize_lark_onboard".into(),
        description: "Finalize Lark channel onboarding after credentials have been stored and \
            the connection tested. This saves the channel configuration, enables the Feishu \
            icon in the titlebar, and starts the WebSocket connection. Call this as the last \
            step after onboard_lark_store_credentials and onboard_lark_test_connection."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {}
        }),
        category: "channels".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::OnboardingFinalize {
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({ "error": "Timed out after 10s" }))
                }
            }
        })
    });

    registry.register(skill, handler);
}
