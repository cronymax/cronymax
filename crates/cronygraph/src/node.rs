//! Agent node — self-contained spawnable agent unit.
//!
//! An [`AgentNode`] wraps `AgentLoopRunner` with a name and optional model override,
//! making agents first-class composable units that can be spawned by the orchestrator.

use std::collections::HashMap;

use crate::engine::{
    AgentLoopConfig, AgentLoopResult, AgentLoopRunner, LlmBackend, ToolExecutor,
};
use crate::types::SkillHandler;
use crate::types::ChatMessage;

// ─── AgentNode ───────────────────────────────────────────────────────────────

/// A single agent node — self-contained unit for running one agent.
///
/// The framework provides the base `AgentNode` with name, system prompt, tools,
/// and handlers. Application code can construct nodes directly or use
/// higher-level builders (e.g., from agent manifests).
pub struct AgentNode {
    /// Unique agent name.
    pub name: String,
    /// System prompt for this agent.
    pub system_prompt: String,
    /// Tool definitions in OpenAI JSON format.
    pub tools: Vec<serde_json::Value>,
    /// Skill handler lookup by tool function name.
    pub skill_handlers: HashMap<String, SkillHandler>,
    /// Agent loop configuration.
    pub config: AgentLoopConfig,
    /// Override model for this agent (e.g., cheap model for triage).
    pub model: Option<String>,
}

impl AgentNode {
    /// Create a new agent node with the given configuration.
    pub fn new(
        name: impl Into<String>,
        system_prompt: impl Into<String>,
        config: AgentLoopConfig,
    ) -> Self {
        Self {
            name: name.into(),
            system_prompt: system_prompt.into(),
            tools: Vec::new(),
            skill_handlers: HashMap::new(),
            config,
            model: None,
        }
    }

    /// Set the model override for this agent.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Add a tool definition and its handler.
    pub fn with_tool(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
        handler: SkillHandler,
    ) -> Self {
        let name = name.into();
        self.tools.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description.into(),
                "parameters": parameters,
            }
        }));
        self.skill_handlers.insert(name, handler);
        self
    }

    /// Add pre-built tool definitions and handlers in bulk.
    pub fn with_tools(
        mut self,
        tools: Vec<serde_json::Value>,
        handlers: HashMap<String, SkillHandler>,
    ) -> Self {
        self.tools.extend(tools);
        self.skill_handlers.extend(handlers);
        self
    }

    /// Run this agent to completion with the given LLM backend and tool executor.
    pub async fn run(
        &self,
        llm: &dyn LlmBackend,
        tool_executor: &dyn ToolExecutor,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<AgentLoopResult> {
        let runner = AgentLoopRunner::new(
            self.config.clone(),
            self.system_prompt.clone(),
            self.tools.clone(),
            self.skill_handlers.clone(),
        );
        runner.run(llm, tool_executor, messages).await
    }
}
