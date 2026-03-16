//! Scheduler skills — CRUD for cron-based scheduled tasks.
//!
//! All operations relay through AppEvent → app.rs → ScheduledTaskStore,
//! returning results via PendingResultMap oneshots.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::{AppEvent, PendingResultMap};

/// Register all scheduler skills into the registry.
pub fn register_scheduler_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    register_create_schedule_task(registry, proxy.clone(), pending_results.clone());
    register_list_schedule_tasks(registry, proxy.clone(), pending_results.clone());
    register_get_schedule_task(registry, proxy.clone(), pending_results.clone());
    register_delete_schedule_task(registry, proxy.clone(), pending_results.clone());
    register_toggle_schedule_task(registry, proxy.clone(), pending_results.clone());
    register_update_schedule_task(registry, proxy, pending_results);
}

// ─── create_schedule_task ────────────────────────────────────────────────────

fn register_create_schedule_task(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.scheduler.create".into(),
        description:
            "Create a new scheduled task. Use 'delay_minutes' for a one-shot delayed task \
            (e.g., 'run this once after 5 minutes'), or 'cron' for recurring schedules. \
            Use action_type='prompt' for tasks that need AI reasoning or multi-step workflows \
            (e.g., fetch data AND send to a channel). Use action_type='command' only for simple \
            shell commands whose output you don't need to process further."
                .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable task name"
                },
                "cron": {
                    "type": "string",
                    "description": "5-field POSIX cron expression (e.g., '0 9 * * *' for daily at 9am). Omit if using delay_minutes."
                },
                "delay_minutes": {
                    "type": "integer",
                    "description": "Run the task once after this many minutes from now. Mutually exclusive with cron."
                },
                "action_type": {
                    "type": "string",
                    "enum": ["prompt", "command"],
                    "description": "Type of action: 'prompt' sends text to AI, 'command' executes shell command"
                },
                "action_value": {
                    "type": "string",
                    "description": "The prompt text or shell command to execute"
                },
                "agent_name": {
                    "type": "string",
                    "description": "Optional agent name to invoke (empty = none)"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Whether the task starts enabled (default: true)"
                },
                "run_once": {
                    "type": "boolean",
                    "description": "If true, auto-disable after first execution (default: false). Automatically set when using delay_minutes."
                }
            },
            "required": ["name", "action_type", "action_value"]
        }),
        category: "scheduler".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let name = args["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'name'"))?
                .to_string();
            let action_type = args["action_type"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'action_type'"))?
                .to_string();
            let action_value = args["action_value"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'action_value'"))?
                .to_string();
            let agent_name = args["agent_name"].as_str().unwrap_or("").to_string();
            let enabled = args["enabled"].as_bool().unwrap_or(true);

            // Resolve cron vs delay_minutes.
            let delay_minutes = args["delay_minutes"].as_u64();
            let explicit_cron = args["cron"].as_str().map(|s| s.to_string());

            let (cron, run_once) = if let Some(mins) = delay_minutes {
                // Convert delay to a one-shot cron: compute the target time.
                let target = chrono::Local::now() + chrono::Duration::minutes(mins as i64);
                let cron_expr = format!(
                    "{} {} {} {} *",
                    target.format("%M"),
                    target.format("%H"),
                    target.format("%d"),
                    target.format("%m"),
                );
                (cron_expr, true)
            } else if let Some(cron) = explicit_cron {
                let run_once = args["run_once"].as_bool().unwrap_or(false);
                (cron, run_once)
            } else {
                return Ok(json!({ "error": "Either 'cron' or 'delay_minutes' is required" }));
            };

            // Validate cron expression before sending to main thread.
            if let Err(msg) = crate::ai::scheduler::validate_cron(&cron) {
                return Ok(json!({ "error": msg }));
            }

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::SchedulerCreate {
                name,
                cron,
                action_type,
                action_value,
                agent_name,
                enabled,
                run_once,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(val)) => Ok(val),
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

// ─── list_schedule_tasks ─────────────────────────────────────────────────────

fn register_list_schedule_tasks(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.scheduler.list".into(),
        description: "List all scheduled tasks with their current status, cron expressions, \
            and enabled state."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {}
        }),
        category: "scheduler".into(),
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

            let _ = proxy.send_event(AppEvent::SchedulerList {
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(val)) => Ok(val),
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

// ─── get_schedule_task ───────────────────────────────────────────────────────

fn register_get_schedule_task(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.scheduler.get".into(),
        description: "Get detailed information about a single scheduled task by ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to look up"
                }
            },
            "required": ["task_id"]
        }),
        category: "scheduler".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let task_id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'task_id'"))?
                .to_string();

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::SchedulerGet {
                task_id,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(val)) => Ok(val),
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

// ─── delete_schedule_task ────────────────────────────────────────────────────

fn register_delete_schedule_task(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.scheduler.delete".into(),
        description: "Delete a scheduled task by ID. This is permanent.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to delete"
                }
            },
            "required": ["task_id"]
        }),
        category: "scheduler".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let task_id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'task_id'"))?
                .to_string();

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::SchedulerDelete {
                task_id,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(val)) => Ok(val),
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

// ─── toggle_schedule_task ────────────────────────────────────────────────────

fn register_toggle_schedule_task(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.scheduler.toggle".into(),
        description: "Toggle a scheduled task's enabled/disabled state.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to toggle"
                }
            },
            "required": ["task_id"]
        }),
        category: "scheduler".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let task_id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'task_id'"))?
                .to_string();

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::SchedulerToggle {
                task_id,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(val)) => Ok(val),
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

// ─── update_schedule_task ────────────────────────────────────────────────────

fn register_update_schedule_task(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.scheduler.update".into(),
        description: "Update one or more fields of an existing scheduled task.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to update"
                },
                "name": {
                    "type": "string",
                    "description": "New task name"
                },
                "cron": {
                    "type": "string",
                    "description": "New cron expression"
                },
                "action_type": {
                    "type": "string",
                    "enum": ["prompt", "command"],
                    "description": "New action type"
                },
                "action_value": {
                    "type": "string",
                    "description": "New action value"
                },
                "agent_name": {
                    "type": "string",
                    "description": "New agent name"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "New enabled state"
                }
            },
            "required": ["task_id"]
        }),
        category: "scheduler".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let task_id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'task_id'"))?
                .to_string();

            // Validate cron if provided.
            if let Some(cron) = args["cron"].as_str()
                && let Err(msg) = crate::ai::scheduler::validate_cron(cron)
            {
                return Ok(json!({ "error": msg }));
            }

            let name = args["name"].as_str().map(|s| s.to_string());
            let cron = args["cron"].as_str().map(|s| s.to_string());
            let action_type = args["action_type"].as_str().map(|s| s.to_string());
            let action_value = args["action_value"].as_str().map(|s| s.to_string());
            let agent_name = args["agent_name"].as_str().map(|s| s.to_string());
            let enabled = args["enabled"].as_bool();

            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::SchedulerUpdate {
                task_id,
                name,
                cron,
                action_type,
                action_value,
                agent_name,
                enabled,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                Ok(Ok(val)) => Ok(val),
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
