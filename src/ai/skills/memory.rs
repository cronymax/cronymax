use super::*;
use crate::ai::db::DbStore;

// ─── Persistent Memory (SQLite FTS5, moved from internal.rs) ─────────────────

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub(super) fn register_save_memory_persistent(
    registry: &mut SkillRegistry,
    db: DbStore,
    profile_id: String,
) {
    let skill = Skill {
        name: "cronymax.chat.save_memory_persistent".into(),
        description: "Save a memory entry to persistent SQLite storage with full-text search. \
            Use this for important facts, user preferences, project notes, and instructions \
            that should survive across sessions."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Memory content to persist"
                },
                "tag": {
                    "type": "string",
                    "enum": ["general", "project", "preference", "fact", "instruction", "context"],
                    "description": "Category tag for the memory"
                },
                "pinned": {
                    "type": "boolean",
                    "description": "Pin this memory to prevent LRU eviction (default: false)"
                }
            },
            "required": ["content"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let db = db.clone();
        let pid = profile_id.clone();
        Box::pin(async move {
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;
            let tag = args["tag"].as_str().unwrap_or("general");
            let pinned = args["pinned"].as_bool().unwrap_or(false);
            let token_count = (content.chars().count().div_ceil(4)) as i64;
            let now = now_millis();

            let row = crate::ai::db::MemoryRow {
                id: 0,
                profile_id: pid,
                content: content.to_string(),
                tag: tag.to_string(),
                pinned,
                token_count,
                created_at: now,
                last_used_at: now,
                access_count: 0,
                embedding: None,
            };

            let id = db.memory_insert(&row)?;
            let _ = db.memory_evict(&row.profile_id, 500);

            Ok(json!({
                "id": id,
                "message": format!("Memory saved (id={}, tag={})", id, tag)
            }))
        })
    });

    registry.register(skill, handler);
}

pub(super) fn register_search_memory_persistent(
    registry: &mut SkillRegistry,
    db: DbStore,
    profile_id: String,
) {
    let skill = Skill {
        name: "cronymax.chat.search_memory_persistent".into(),
        description: "Search persistent memories using full-text search (FTS5). \
            Returns memories ranked by relevance. Use this to recall facts, preferences, \
            and context across sessions."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (supports prefix matching)"
                },
                "tag": {
                    "type": "string",
                    "enum": ["general", "project", "preference", "fact", "instruction", "context"],
                    "description": "Optional tag filter"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return (default: 10)"
                }
            },
            "required": ["query"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let db = db.clone();
        let pid = profile_id.clone();
        Box::pin(async move {
            let query = args["query"].as_str().unwrap_or("");
            let tag = args["tag"].as_str();
            let limit = args["limit"].as_u64().unwrap_or(10) as usize;

            let results = db.memory_search(&pid, query, tag, limit)?;

            for r in &results {
                let _ = db.memory_touch(r.id);
            }

            let items: Vec<Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "content": r.content,
                        "tag": r.tag,
                        "pinned": r.pinned,
                        "token_count": r.token_count,
                        "access_count": r.access_count,
                    })
                })
                .collect();

            Ok(json!({
                "results": items,
                "count": items.len(),
            }))
        })
    });

    registry.register(skill, handler);
}

pub(super) fn register_list_memories(
    registry: &mut SkillRegistry,
    db: DbStore,
    profile_id: String,
) {
    let skill = Skill {
        name: "cronymax.chat.list_memories".into(),
        description: "List all persistent memories for the current profile, ordered by \
            most recently used. Use this to see what the agent has remembered."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Max entries to return (default: 20)"
                }
            }
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let db = db.clone();
        let pid = profile_id.clone();
        Box::pin(async move {
            let limit = args["limit"].as_u64().unwrap_or(20) as usize;
            let results = db.memory_list(&pid, limit)?;

            let items: Vec<Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "content": r.content,
                        "tag": r.tag,
                        "pinned": r.pinned,
                        "access_count": r.access_count,
                    })
                })
                .collect();

            Ok(json!({
                "memories": items,
                "total": items.len(),
            }))
        })
    });

    registry.register(skill, handler);
}

pub(super) fn register_delete_memory(registry: &mut SkillRegistry, db: DbStore) {
    let skill = Skill {
        name: "cronymax.chat.delete_memory".into(),
        description: "Delete a persistent memory entry by its ID.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "Memory entry ID to delete"
                }
            },
            "required": ["id"]
        }),
        category: "chat".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let db = db.clone();
        Box::pin(async move {
            let id = args["id"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' argument"))?;
            db.memory_delete(id)?;
            Ok(json!({ "message": format!("Memory {} deleted", id) }))
        })
    });

    registry.register(skill, handler);
}
