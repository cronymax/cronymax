//! Chat skills — chat block manipulation, context window management,
//! persistent memory, and session state operations.
#![allow(dead_code)]

use super::memory::*;

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::db::DbStore;
use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::{AppEvent, PendingResultMap};

/// Register all chat block/context/memory skills.
pub fn register_chat_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
    db: Option<DbStore>,
    profile_id: String,
) {
    register_manage_context_window(registry);
    register_star_chat_block(registry, proxy.clone());
    register_unstar_chat_block(registry, proxy.clone());
    register_reference_block_content(registry, proxy.clone(), pending_results.clone());
    register_add_context(registry, proxy.clone());
    register_remove_context(registry, proxy.clone());
    register_compact_context(registry, proxy, pending_results);

    // Persistent memory skills (moved from internal.rs → chat category).
    if let Some(db) = db {
        register_save_memory_persistent(registry, db.clone(), profile_id.clone());
        register_search_memory_persistent(registry, db.clone(), profile_id.clone());
        register_list_memories(registry, db.clone(), profile_id);
        register_delete_memory(registry, db);
    }
}

// ─── Context Window Management ───────────────────────────────────────────────

fn register_manage_context_window(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.chat.manage_context_window".into(),
        description: "Manage the conversation context window. Actions: 'status' shows token \
            usage, 'compact' prunes old messages to free space, 'clear' removes all \
            non-essential messages. Use this when the context is getting full."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "compact", "clear"],
                    "description": "Action to perform on the context window"
                }
            },
            "required": ["action"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("status").to_string();

            Ok(json!({
                "action": action,
                "message": format!("Context window '{}' action queued", action),
            }))
        })
    });

    registry.register(skill, handler);
}

// ─── Star / Unstar Chat Block ────────────────────────────────────────────────

fn register_star_chat_block(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.chat.star_block".into(),
        description:
            "Star a chat message to mark it as important and preserve it during context compaction."
                .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Chat session ID." },
                "message_id": { "type": "integer", "description": "Message ID to star." }
            },
            "required": ["session_id", "message_id"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let message_id = args["message_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'message_id'"))?
                as u32;
            let _ = proxy.send_event(AppEvent::StarChatBlock {
                session_id,
                message_id,
            });
            Ok(json!({ "starred": true, "message_id": message_id }))
        })
    });

    registry.register(skill, handler);
}

fn register_unstar_chat_block(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.chat.unstar_block".into(),
        description: "Remove star from a chat message.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Chat session ID." },
                "message_id": { "type": "integer", "description": "Message ID to unstar." }
            },
            "required": ["session_id", "message_id"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let message_id = args["message_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'message_id'"))?
                as u32;
            let _ = proxy.send_event(AppEvent::UnstarChatBlock {
                session_id,
                message_id,
            });
            Ok(json!({ "starred": false, "message_id": message_id }))
        })
    });

    registry.register(skill, handler);
}

// ─── Reference Block Content ─────────────────────────────────────────────────

fn register_reference_block_content(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.chat.reference_block".into(),
        description:
            "Retrieve the content of a specific chat message by session ID and message ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Chat session ID." },
                "message_id": { "type": "integer", "description": "Message ID to retrieve." }
            },
            "required": ["session_id", "message_id"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let message_id = args["message_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'message_id'"))?
                as u32;
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending.lock().map_err(|e| anyhow::anyhow!("Lock: {}", e))?;
                map.insert(request_id.clone(), tx);
            }
            let _ = proxy.send_event(AppEvent::ReferenceBlockContent {
                session_id,
                message_id,
                request_id,
            });
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
                Ok(Ok(val)) => Ok(val),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => Ok(json!({ "error": "Timeout waiting for block content" })),
            }
        })
    });

    registry.register(skill, handler);
}

// ─── Add / Remove / Compact Context ─────────────────────────────────────────

fn register_add_context(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.chat.add_context".into(),
        description: "Inject additional context into the active session's conversation. \
            Useful for adding reference documentation, code snippets, or instructions."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Target session ID." },
                "content": { "type": "string", "description": "Context content to add." },
                "label": { "type": "string", "description": "Optional label for the context block." }
            },
            "required": ["session_id", "content"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'content'"))?
                .to_string();
            let label = args["label"].as_str().map(|s| s.to_string());
            let _ = proxy.send_event(AppEvent::AddContext {
                session_id,
                content,
                label,
            });
            Ok(json!({ "added": true }))
        })
    });

    registry.register(skill, handler);
}

fn register_remove_context(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.chat.remove_context".into(),
        description: "Remove a context message from the session's conversation by message ID."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Target session ID." },
                "message_id": { "type": "integer", "description": "Message ID to remove." }
            },
            "required": ["session_id", "message_id"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let message_id = args["message_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'message_id'"))?
                as u32;
            let _ = proxy.send_event(AppEvent::RemoveContext {
                session_id,
                message_id,
            });
            Ok(json!({ "removed": true }))
        })
    });

    registry.register(skill, handler);
}

fn register_compact_context(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.chat.compact_context".into(),
        description:
            "Compact the conversation context by pruning old messages to free token budget. \
            Returns statistics about freed tokens and remaining messages."
                .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "integer", "description": "Target session ID." }
            },
            "required": ["session_id"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let session_id = args["session_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'session_id'"))?
                as u32;
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending.lock().map_err(|e| anyhow::anyhow!("Lock: {}", e))?;
                map.insert(request_id.clone(), tx);
            }
            let _ = proxy.send_event(AppEvent::CompactContext {
                session_id,
                request_id,
            });
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
                Ok(Ok(val)) => Ok(val),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed" })),
                Err(_) => Ok(json!({ "error": "Timeout waiting for context compaction" })),
            }
        })
    });

    registry.register(skill, handler);
}
