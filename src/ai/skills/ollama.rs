// Ollama management skills — AI-invocable tools for model management.

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::client::ollama_manager::OllamaManager;
use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::AppEvent;

/// Register all Ollama management skills.
pub fn register_ollama_skills(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    register_ollama_list(registry);
    register_ollama_pull(registry, proxy.clone());
    register_ollama_remove(registry);
    register_ollama_status(registry);
}

fn register_ollama_list(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.ollama.list".into(),
        description:
            "List all locally available Ollama models with their names, sizes, and details.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
        category: "ollama".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        Box::pin(async move {
            let manager = OllamaManager::default();
            let models = manager.list_models().await?;
            let items: Vec<Value> = models
                .iter()
                .map(|m| {
                    json!({
                        "name": m.name,
                        "size_mb": m.size / (1024 * 1024),
                        "family": m.family(),
                        "parameter_size": m.parameter_size(),
                        "quantization_level": m.quantization_level(),
                    })
                })
                .collect();
            Ok(json!({"models": items, "count": items.len()}))
        })
    });
    registry.register(skill, handler);
}

fn register_ollama_pull(registry: &mut SkillRegistry, proxy: EventLoopProxy<AppEvent>) {
    let skill = Skill {
        name: "cronymax.ollama.pull".into(),
        description: "Download a model from the Ollama registry. Progress is shown in the chat."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "description": "Model name to pull, e.g., 'llama3', 'codellama:13b', 'mistral'"
                }
            },
            "required": ["model"]
        }),
        category: "ollama".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        Box::pin(async move {
            let model = args["model"].as_str().unwrap_or("").to_string();
            if model.is_empty() {
                return Ok(json!({"error": "model name is required"}));
            }
            let manager = OllamaManager::default();
            manager.pull_model(&model, proxy, None).await;
            Ok(json!({"status": "pull_started", "model": model}))
        })
    });
    registry.register(skill, handler);
}

fn register_ollama_remove(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.ollama.remove".into(),
        description: "Delete a locally downloaded Ollama model.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "description": "Model name to remove"
                }
            },
            "required": ["model"]
        }),
        category: "ollama".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        Box::pin(async move {
            let model = args["model"].as_str().unwrap_or("").to_string();
            if model.is_empty() {
                return Ok(json!({"error": "model name is required"}));
            }
            let manager = OllamaManager::default();
            manager.remove_model(&model).await?;
            Ok(json!({"status": "removed", "model": model}))
        })
    });
    registry.register(skill, handler);
}

fn register_ollama_status(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.ollama.status".into(),
        description: "Check Ollama daemon status: whether running, version, and loaded models."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
        category: "ollama".into(),
    };

    let handler: SkillHandler = Arc::new(move |_args: Value| {
        Box::pin(async move {
            let manager = OllamaManager::default();
            let status = manager.show_status().await?;
            Ok(json!({"status": status}))
        })
    });
    registry.register(skill, handler);
}
