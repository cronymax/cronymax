use super::*;

impl SkillRegistry {
    pub fn register_memory_skills(
        &mut self,
        manager: Arc<std::sync::Mutex<crate::profile::ProfileManager>>,
    ) {
        // save_memory
        {
            let skill = Skill {
                name: "cronymax.chat.save_memory".into(),
                description: "Save a memory entry for the current profile.".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "Memory content to save"
                        },
                        "tag": {
                            "type": "string",
                            "description": "Memory tag: general, project, preference, fact, instruction, context"
                        },
                        "pinned": {
                            "type": "boolean",
                            "description": "Whether to pin this memory (prevents LRU eviction)"
                        }
                    },
                    "required": ["content"]
                }),
                category: "chat".into(),
            };

            let mgr = manager.clone();
            let handler: SkillHandler = Arc::new(move |args: Value| {
                let mgr = mgr.clone();
                Box::pin(async move {
                    let content = args["content"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;
                    let tag_str = args["tag"].as_str().unwrap_or("general");
                    let pinned = args["pinned"].as_bool().unwrap_or(false);

                    let tag = match tag_str {
                        "project" => crate::profile::MemoryTag::Project,
                        "preference" => crate::profile::MemoryTag::Preference,
                        "fact" => crate::profile::MemoryTag::Fact,
                        "instruction" => crate::profile::MemoryTag::Instruction,
                        "context" => crate::profile::MemoryTag::Context,
                        _ => crate::profile::MemoryTag::General,
                    };

                    let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    let token_count = content.chars().count().div_ceil(4); // Heuristic.

                    let entry = crate::profile::MemoryEntry {
                        id: id.clone(),
                        content: content.to_string(),
                        tag,
                        pinned,
                        token_count,
                        created_at: now,
                        last_used_at: now,
                        access_count: 0,
                    };

                    let mut mgr = mgr
                        .lock()
                        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                    if let Some(memory) = mgr.memory_mut() {
                        memory.insert(entry);
                    }
                    let _ = mgr.save_memory();

                    Ok(json!({
                        "id": id,
                        "message": "Memory saved"
                    }))
                })
            });

            self.register(skill, handler);
        }

        // recall_memory
        {
            let skill = Skill {
                name: "cronymax.chat.recall_memory".into(),
                description: "Search and recall memory entries for the current profile.".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "tag": {
                            "type": "string",
                            "description": "Filter by tag (optional)"
                        }
                    },
                    "required": ["query"]
                }),
                category: "chat".into(),
            };

            let mgr = manager;
            let handler: SkillHandler = Arc::new(move |args: Value| {
                let mgr = mgr.clone();
                Box::pin(async move {
                    let query = args["query"].as_str().unwrap_or("");
                    let tag_str = args["tag"].as_str();

                    let tag = tag_str.map(|s| match s {
                        "project" => crate::profile::MemoryTag::Project,
                        "preference" => crate::profile::MemoryTag::Preference,
                        "fact" => crate::profile::MemoryTag::Fact,
                        "instruction" => crate::profile::MemoryTag::Instruction,
                        "context" => crate::profile::MemoryTag::Context,
                        _ => crate::profile::MemoryTag::General,
                    });

                    let mut mgr = mgr
                        .lock()
                        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

                    let results: Vec<Value> = if let Some(memory) = mgr.memory() {
                        let entries = memory.search(query, tag.as_ref());
                        entries
                            .iter()
                            .map(|e| {
                                json!({
                                    "id": e.id,
                                    "content": e.content,
                                    "tag": format!("{:?}", e.tag),
                                    "pinned": e.pinned,
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };

                    // Touch accessed entries.
                    if let Some(memory) = mgr.memory_mut() {
                        for r in &results {
                            if let Some(id) = r["id"].as_str() {
                                memory.touch(id);
                            }
                        }
                    }
                    let _ = mgr.save_memory();

                    Ok(json!({ "results": results }))
                })
            });

            self.register(skill, handler);
        }
    }

    /// Register UI skills (webview, terminal, tab, browser, general, onboarding)
    /// that dispatch actions to the main thread via the event loop proxy.
    pub fn register_ui_skills(&mut self, deps: &SkillDependencies) {
        webview::register_webview_skills(self, deps.proxy.clone(), deps.webview_info.clone());
        terminal::register_terminal_skills(
            self,
            deps.proxy.clone(),
            deps.pending_results.clone(),
            deps.terminal_info.clone(),
        );
        tab::register_tab_skills(self, deps.proxy.clone(), deps.tab_info.clone());
        browser::register_browser_skills(self, deps.proxy.clone(), deps.pending_results.clone());
        general::register_general_skills(self, deps.proxy.clone(), deps.pending_results.clone());
        onboarding::register_onboarding_skills(
            self,
            deps.proxy.clone(),
            deps.pending_results.clone(),
            deps.onboarding_state.clone(),
        );
        chat::register_chat_skills(
            self,
            deps.proxy.clone(),
            deps.pending_results.clone(),
            deps.db.clone(),
            deps.profile_id.clone(),
        );
        scheduler::register_scheduler_skills(
            self,
            deps.proxy.clone(),
            deps.pending_results.clone(),
        );
        ollama::register_ollama_skills(self, deps.proxy.clone());
        if let Some(ref secret_store) = deps.secret_store {
            credentials::register_credential_skills(self, secret_store.clone());
        }
    }
}
