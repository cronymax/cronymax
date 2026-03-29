// Delegation skill — allows the lead agent to spawn sub-agents as tool calls.
//
// Inspired by DeerFlow's supervisor pattern where the lead agent can invoke
// specialist sub-agents. The sub-agent runs its own tool-calling loop to
// completion and returns its response + chain-of-thought summary.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::ai::agent::AgentRegistry;
use crate::ai::agent_loop::{
    AgentLoopConfig, NonStreamingLlmBackend, SequentialToolExecutor,
};
use crate::ai::context::{ChatMessage, MessageImportance, MessageRole};
use crate::ai::orchestration::{AgentNode, AgentNodeExt, DelegationRequest};
use crate::ai::skills::{Skill, SkillHandler};

/// Build the `delegate_to_agent` skill definition.
pub fn delegation_skill() -> Skill {
    Skill {
        name: "delegate_to_agent".into(),
        description: "Delegate a task to a specialist agent. The agent runs its own \
                      tool-calling loop to completion and returns its response. \
                      Use this when the task requires specialized knowledge or tools \
                      that a specific installed agent provides."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "agent_name": {
                    "type": "string",
                    "description": "Name of the installed agent to invoke"
                },
                "task": {
                    "type": "string",
                    "description": "Task description to send to the agent"
                },
                "constraints": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional constraints for the agent (e.g., 'only modify test files')"
                },
                "output_format": {
                    "type": "string",
                    "description": "Optional expected output format (e.g., 'JSON array')"
                }
            },
            "required": ["agent_name", "task"]
        }),
        category: "general".into(),
    }
}

/// Dependencies needed by the delegation handler.
pub struct DelegationDeps {
    /// Agent registry for looking up installed agents.
    pub agent_registry: Arc<std::sync::Mutex<AgentRegistry>>,
    /// OpenAI client for the sub-agent LLM calls.
    pub openai_client: async_openai::Client<async_openai::config::OpenAIConfig>,
    /// Default model for sub-agents.
    pub model: String,
    /// Base skill handlers that sub-agents can use.
    pub base_handlers: HashMap<String, SkillHandler>,
    /// Agent loop config for sub-agents.
    pub config: AgentLoopConfig,
    /// Current delegation depth (used to enforce max_delegation_depth).
    pub current_depth: u32,
    /// Maximum delegation depth.
    pub max_delegation_depth: u32,
}

/// Build the delegation handler.
///
/// The handler is a closure that captures the dependencies and can be
/// registered in the skill registry.
pub fn delegation_handler(deps: Arc<DelegationDeps>) -> SkillHandler {
    Arc::new(move |args: Value| {
        let deps = deps.clone();
        Box::pin(async move {
            // Parse the delegation request from tool call arguments.
            let agent_name = args["agent_name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'agent_name'"))?
                .to_string();
            let task = args["task"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'task'"))?
                .to_string();
            let constraints: Vec<String> = args["constraints"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let output_format = args["output_format"].as_str().map(String::from);

            // Check delegation depth.
            if deps.current_depth >= deps.max_delegation_depth {
                return Ok(json!({
                    "error": format!(
                        "Maximum delegation depth ({}) reached. Cannot delegate further.",
                        deps.max_delegation_depth
                    )
                }));
            }

            // Look up the agent manifest.
            let manifest = {
                let registry = deps.agent_registry.lock()
                    .map_err(|e| anyhow::anyhow!("Failed to lock agent registry: {}", e))?;
                match registry.lookup(&agent_name) {
                    Some(m) => m.clone(),
                    None => {
                        return Ok(json!({
                            "error": format!("Agent '{}' is not installed", agent_name)
                        }));
                    }
                }
            };

            if !manifest.agent.enabled {
                return Ok(json!({
                    "error": format!("Agent '{}' is disabled", agent_name)
                }));
            }

            // Build the delegation request.
            let request = DelegationRequest {
                agent_name: agent_name.clone(),
                task,
                constraints,
                output_format,
            };

            // Construct the AgentNode.
            let node = AgentNode::from_manifest(
                &manifest,
                &deps.base_handlers,
                deps.config.clone(),
            );

            // Build messages for the sub-agent.
            let mut messages = Vec::new();
            messages.push(ChatMessage::new(
                MessageRole::System,
                node.system_prompt.clone(),
                MessageImportance::System,
                0,
            ));
            messages.push(request.to_user_message());

            // Create the LLM backend.
            let model = node.model.as_deref().unwrap_or(&deps.model);
            let llm = NonStreamingLlmBackend {
                client: deps.openai_client.clone(),
                model: model.to_string(),
            };

            // Run the sub-agent.
            log::info!(
                "[Delegation] Delegating to agent '{}' (depth {}/{})",
                agent_name,
                deps.current_depth + 1,
                deps.max_delegation_depth
            );

            match node.run(&llm, &SequentialToolExecutor, messages).await {
                Ok(result) => {
                    let usage_info = result.total_usage.as_ref().map(|u| {
                        json!({
                            "prompt_tokens": u.prompt_tokens,
                            "completion_tokens": u.completion_tokens,
                            "total_tokens": u.total_tokens,
                        })
                    });

                    Ok(json!({
                        "status": "completed",
                        "agent": agent_name,
                        "response": result.response,
                        "usage": usage_info,
                    }))
                }
                Err(e) => {
                    log::warn!("[Delegation] Agent '{}' failed: {}", agent_name, e);
                    Ok(json!({
                        "status": "failed",
                        "agent": agent_name,
                        "error": e.to_string(),
                    }))
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegation_skill_definition_is_valid() {
        let skill = delegation_skill();
        assert_eq!(skill.name, "delegate_to_agent");
        assert_eq!(skill.category, "general");

        // Verify the parameters schema is valid JSON.
        let params = &skill.parameters_schema;
        assert_eq!(params["type"], "object");
        let required = params["required"].as_array().unwrap();
        assert!(required.contains(&json!("agent_name")));
        assert!(required.contains(&json!("task")));
    }
}
