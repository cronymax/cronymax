//! Chat history + tool primitives shared by every provider.
//!
//! The shape mirrors the OpenAI chat-completions wire format closely
//! because that's what the renderer was already producing
//! (`web/src/agent_runtime/llm.js`) and what every "OpenAI-compatible"
//! gateway expects. Other providers can adapt to/from this shape in
//! their own modules.

use serde::{Deserialize, Serialize};

/// Speaker role for a [`ChatMessage`]. `Tool` carries the result of a
/// tool call back into the next LLM turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A single chat-history entry. `content` is optional because an
/// assistant turn that issues only tool calls has no textual payload;
/// tool entries instead use `tool_call_id` to bind back to the call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<String>,
    /// Assistant-emitted tool calls. Empty for non-assistant roles.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    /// Set on `role: tool` messages — the id of the call this is the
    /// result of.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
    /// Optional human-friendly speaker name (mostly used for tool
    /// messages to surface the tool's name).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: None,
            tool_calls: calls,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_result(call_id: impl Into<String>, name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            name: Some(name.into()),
        }
    }
}

/// One tool invocation requested by the model. `arguments` is the raw
/// JSON string the model produced; the loop validates/parses it before
/// dispatch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// JSON-schema-flavoured tool advertisement. Mirrors OpenAI's
/// `{ type: "function", function: { name, description, parameters } }`
/// shape after the wire-level wrapper is unwrapped.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments object.
    pub parameters: serde_json::Value,
}

/// Inbound request to the provider. `tools` may be empty for plain
/// chat completion.
#[derive(Clone, Debug)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDef>,
    /// Sampling temperature (`None` lets the provider apply its own
    /// default).
    pub temperature: Option<f32>,
}

impl LlmRequest {
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: Vec::new(),
            temperature: None,
        }
    }

    pub fn with_tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.tools = tools;
        self
    }
}

/// Why a streaming turn ended. `Other` carries any provider-specific
/// reason the runtime doesn't have a strong opinion on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Other(String),
}

impl FinishReason {
    pub(crate) fn from_str(s: &str) -> Self {
        match s {
            "stop" => Self::Stop,
            "tool_calls" => Self::ToolCalls,
            "length" => Self::Length,
            other => Self::Other(other.to_owned()),
        }
    }
}
