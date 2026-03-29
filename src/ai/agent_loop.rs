//! Unified agent loop — shared abstraction for both interactive and channel paths.
//!
//! # Design
//!
//! Both the interactive chat (streaming, event-driven) and channel agent loop
//! (non-streaming, inline) share the same core cycle:
//!
//!   **Context Build → Middleware (PRE) → LLM → Middleware (POST) → Tool Exec → Loop**
//!
//! Core engine types (AgentLoopConfig, LlmBackend, ToolExecutor, MemoryBackend, etc.)
//! are defined in the `cronygraph` crate and re-exported here for backward compatibility.
//!
//! This module provides the OpenAI-specific `NonStreamingLlmBackend` and `complete_chat()`
//! implementations, plus the channel pipeline orchestration.
#![allow(dead_code)]

use std::collections::HashMap;

use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessage, ChatCompletionRequestToolMessage,
        ChatCompletionRequestUserMessage, ChatCompletionTool, ChatCompletionToolType,
        CreateChatCompletionRequestArgs, FunctionObject,
    },
};
use async_trait::async_trait;
use serde_json::Value;

use crate::ai::context::{ChatMessage, MessageImportance, MessageRole};

use crate::ai::skills::SkillHandler;
use crate::ai::stream::{TokenUsage, ToolCallInfo};

// Re-export core engine types from cronygraph.
pub use cronygraph::engine::{
    AgentLoopConfig, AgentLoopResult, AgentLoopRunner, LlmBackend, LlmResult, MemoryBackend,
    ParallelToolExecutor, SequentialToolExecutor, ToolExecutor,
};

// ─── Non-streaming LLM Backend ───────────────────────────────────────────────

/// Make a non-streaming LLM chat completion request and return the full result.
///
/// Converts `ChatMessage` objects to OpenAI request messages and tool JSON
/// definitions to typed structs, then calls the non-streaming API.
async fn complete_chat(
    client: &Client<OpenAIConfig>,
    model: &str,
    messages: &[ChatMessage],
    tools: Option<&[Value]>,
) -> anyhow::Result<(String, Vec<ToolCallInfo>, Option<TokenUsage>)> {
    // Convert ChatMessages → OpenAI request messages.
    let oai_messages: Vec<ChatCompletionRequestMessage> = messages
        .iter()
        .map(|m| match m.role {
            MessageRole::System => {
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: async_openai::types::ChatCompletionRequestSystemMessageContent::Text(
                        m.content.clone(),
                    ),
                    name: None,
                })
            }
            MessageRole::User => {
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        m.content.clone(),
                    ),
                    name: None,
                })
            }
            MessageRole::Assistant => {
                let oai_tc: Option<Vec<async_openai::types::ChatCompletionMessageToolCall>> =
                    if m.tool_calls.is_empty() {
                        None
                    } else {
                        Some(
                            m.tool_calls
                                .iter()
                                .map(|tc| async_openai::types::ChatCompletionMessageToolCall {
                                    id: tc.id.clone(),
                                    r#type: ChatCompletionToolType::Function,
                                    function: async_openai::types::FunctionCall {
                                        name: tc.function_name.clone(),
                                        arguments: tc.arguments.clone(),
                                    },
                                })
                                .collect(),
                        )
                    };
                let content = if m.content.is_empty() && oai_tc.is_some() {
                    None
                } else {
                    Some(
                        async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(
                            m.content.clone(),
                        ),
                    )
                };
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content,
                    name: None,
                    tool_calls: oai_tc,
                    refusal: None,
                    audio: None,
                    #[allow(deprecated)]
                    function_call: None,
                })
            }
            MessageRole::Tool => {
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: async_openai::types::ChatCompletionRequestToolMessageContent::Text(
                        m.content.clone(),
                    ),
                    tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                })
            }
            // Info messages should be filtered before reaching API; fallback to user.
            MessageRole::Info => {
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        m.content.clone(),
                    ),
                    name: None,
                })
            }
        })
        .collect();

    // Convert JSON tool definitions → typed ChatCompletionTool structs.
    let oai_tools: Option<Vec<ChatCompletionTool>> =
        tools.filter(|t| !t.is_empty()).map(|tool_defs| {
            tool_defs
                .iter()
                .filter_map(|t| {
                    let func = t.get("function")?;
                    Some(ChatCompletionTool {
                        r#type: ChatCompletionToolType::Function,
                        function: FunctionObject {
                            name: func.get("name")?.as_str()?.to_string(),
                            description: func
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                            parameters: func.get("parameters").cloned(),
                            strict: None,
                        },
                    })
                })
                .collect()
        });

    let mut builder = CreateChatCompletionRequestArgs::default();
    builder.model(model).messages(oai_messages);
    if let Some(tools) = oai_tools
        && !tools.is_empty()
    {
        builder.tools(tools);
    }
    let request = builder.build()?;

    // Call the non-streaming API.
    let response = client.chat().create(request).await?;

    let choice = response
        .choices
        .first()
        .ok_or_else(|| anyhow::anyhow!("No choices in LLM response"))?;

    let content = choice.message.content.clone().unwrap_or_default();
    let tool_calls = choice
        .message
        .tool_calls
        .as_ref()
        .map(|tcs| {
            tcs.iter()
                .map(|tc| ToolCallInfo {
                    id: tc.id.clone(),
                    function_name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let usage = response.usage.map(|u| TokenUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    });

    Ok((content, tool_calls, usage))
}

/// Non-streaming LLM backend using the OpenAI-compatible chat API.
///
/// Used by the channel agent loop and the memory extraction agent.
/// Wraps the [`complete_chat`] helper to implement the [`LlmBackend`] trait.
pub struct NonStreamingLlmBackend {
    pub client: Client<OpenAIConfig>,
    pub model: String,
}

#[async_trait]
impl LlmBackend for NonStreamingLlmBackend {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Value]>,
    ) -> anyhow::Result<LlmResult> {
        let (response, tool_calls, usage) =
            complete_chat(&self.client, &self.model, messages, tools).await?;
        Ok(LlmResult {
            response,
            tool_calls,
            usage,
        })
    }
}

// ─── Channel Pipeline Orchestration ──────────────────────────────────────────

use crate::ai::db::memory_store::RagMemoryStore;
use crate::channels::ChannelMessage;

/// Runtime dependencies needed by the channel agent loop pipeline.
pub struct AgentLoopDeps {
    /// Pre-initialized OpenAI-compatible client (cloned from `LlmClient`).
    pub openai_client: Client<OpenAIConfig>,
    /// Model identifier (e.g., `"gpt-4o"`).
    pub model: String,
    /// System prompt for the conversation.
    pub system_prompt: String,
    /// Pre-filtered tool definitions in OpenAI JSON format.
    pub tools: Vec<Value>,
    /// Skill handler lookup by tool function name.
    pub skill_handlers: HashMap<String, SkillHandler>,
}

/// Normalize an incoming `ChannelMessage` into a `ChatMessage` for the LLM context.
pub fn normalize_channel_message(msg: &ChannelMessage) -> ChatMessage {
    let sender_label = msg.sender_name.as_deref().unwrap_or(&msg.sender_id);
    let user_content = format!(
        "[Channel: {} | From: {}]\n{}",
        msg.channel_id, sender_label, msg.content
    );
    let token_count = (user_content.chars().count().div_ceil(4)) as u32;
    ChatMessage::new(
        MessageRole::User,
        user_content,
        MessageImportance::Normal,
        token_count,
    )
}

/// Derive a stable numeric session ID from a channel's (channel_id, chat_id) pair.
pub fn session_id_for_channel(channel_id: &str, chat_id: &str) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    channel_id.hash(&mut hasher);
    chat_id.hash(&mut hasher);
    // Keep in the high range to avoid colliding with terminal session IDs.
    900_000 + (hasher.finish() % 100_000) as u32
}

/// Process a channel message through the full 6-stage pipeline.
///
/// # Stages
/// 1. **Message IN** — Normalize `ChannelMessage` → `ChatMessage`
/// 2. **Memory Recall (CTX)** — Build context window (sliding window + RAG)
/// 3. **LLM (AI)** — Send context to the LLM, collect response
/// 4. **Tools/Skills (EXEC)** — Execute tool calls, loop until no more calls
/// 5. **Memory Save / Compaction (STORE)** — Persist + compact if over budget
/// 6. **Response OUT** — Return response text for sending via channel
///
/// # Returns
/// The response text to send back through the channel, or an error.
pub async fn process_channel_message(
    message: &ChannelMessage,
    config: &AgentLoopConfig,
    memory: &RagMemoryStore,
    deps: &AgentLoopDeps,
) -> anyhow::Result<String> {
    let session_id = session_id_for_channel(&message.channel_id, &message.chat_id);

    // ── Stage 1: Message IN ──────────────────────────────────────────────
    let user_msg = normalize_channel_message(message);
    log::info!(
        "[AgentLoop] Stage 1: Normalized message from {} ({} tokens)",
        message.sender_id,
        user_msg.token_count
    );

    // ── Stage 2: Memory Recall (CTX) ────────────────────────────────────
    let recalled = memory
        .recall(
            session_id,
            &message.content,
            config.sliding_window_n,
            config.rag_top_k,
            config.max_context_tokens,
        )
        .await
        .unwrap_or_else(|e| {
            log::warn!("[AgentLoop] Stage 2: Memory recall failed: {}", e);
            Vec::new()
        });
    log::info!(
        "[AgentLoop] Stage 2: Recalled {} context messages",
        recalled.len()
    );

    // Build the full message array: [system, recalled..., user_msg].
    let mut messages: Vec<ChatMessage> = Vec::new();
    messages.push(ChatMessage::new(
        MessageRole::System,
        deps.system_prompt.clone(),
        MessageImportance::System,
        0,
    ));
    messages.extend(recalled);
    messages.push(user_msg.clone());

    // ── Stages 3 & 4: Delegated to unified AgentLoopRunner ─────────────
    let runner = AgentLoopRunner::new(
        config.clone(),
        deps.system_prompt.clone(),
        deps.tools.clone(),
        deps.skill_handlers.clone(),
    );

    let llm_backend = NonStreamingLlmBackend {
        client: deps.openai_client.clone(),
        model: deps.model.clone(),
    };

    let result = runner
        .run(&llm_backend, &SequentialToolExecutor, messages)
        .await?;

    let response_text = result.response;
    let total_usage = result.total_usage;

    // ── Stage 5: Memory Save / Context Compaction ───────────────────────
    if let Err(e) = memory.save(session_id, &user_msg).await {
        log::warn!("[AgentLoop] Stage 5: Failed to save user message: {}", e);
    }
    let assistant_msg = ChatMessage::new(
        MessageRole::Assistant,
        response_text.clone(),
        MessageImportance::Normal,
        (response_text.chars().count().div_ceil(4)) as u32,
    );
    if let Err(e) = memory.save(session_id, &assistant_msg).await {
        log::warn!(
            "[AgentLoop] Stage 5: Failed to save assistant message: {}",
            e
        );
    }

    // Check compaction threshold.
    let total_tokens_used = total_usage.as_ref().map(|u| u.total_tokens).unwrap_or(0) as f32;
    let ratio = total_tokens_used / config.max_context_tokens as f32;

    if ratio >= config.compaction_threshold_hard {
        log::info!(
            "[AgentLoop] Stage 5: Token ratio {:.2} exceeds hard threshold — compacting",
            ratio
        );
        let starred = memory.get_starred(session_id).await.unwrap_or_default();
        let candidates: Vec<ChatMessage> = result
            .messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .cloned()
            .collect();
        match memory
            .compact(session_id, &candidates, &starred, &config.compaction_model)
            .await
        {
            Ok(summary) => {
                log::info!(
                    "[AgentLoop] Compacted {} messages into summary ({} tokens)",
                    candidates.len(),
                    summary.token_count
                );
            }
            Err(e) => {
                log::warn!("[AgentLoop] Compaction failed: {}", e);
            }
        }
    } else if ratio >= config.compaction_threshold_soft {
        log::info!(
            "[AgentLoop] Stage 5: Token ratio {:.2} exceeds soft threshold — async compaction recommended",
            ratio
        );
    }

    // ── Stage 6: Response OUT ───────────────────────────────────────────
    log::info!(
        "[AgentLoop] Stage 6: Returning response ({} chars)",
        response_text.len()
    );
    Ok(response_text)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake LLM backend that returns a canned response.
    struct FakeLlm {
        response: String,
        tool_calls: Vec<ToolCallInfo>,
    }

    #[async_trait]
    impl LlmBackend for FakeLlm {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: Option<&[serde_json::Value]>,
        ) -> anyhow::Result<LlmResult> {
            Ok(LlmResult {
                response: self.response.clone(),
                tool_calls: self.tool_calls.clone(),
                usage: Some(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                }),
            })
        }
    }

    /// Fake LLM that returns tool calls on the first call, plain text on the second.
    struct FakeLlmWithTools {
        calls: std::sync::Mutex<u32>,
    }

    #[async_trait]
    impl LlmBackend for FakeLlmWithTools {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: Option<&[serde_json::Value]>,
        ) -> anyhow::Result<LlmResult> {
            let mut count = self.calls.lock().unwrap();
            *count += 1;
            if *count == 1 {
                Ok(LlmResult {
                    response: "Let me search...".to_string(),
                    tool_calls: vec![ToolCallInfo {
                        id: "tc_1".into(),
                        function_name: "search".into(),
                        arguments: "{\"q\": \"test\"}".into(),
                    }],
                    usage: None,
                })
            } else {
                Ok(LlmResult {
                    response: "Found the answer.".to_string(),
                    tool_calls: vec![],
                    usage: None,
                })
            }
        }
    }

    #[tokio::test]
    async fn runner_returns_direct_response() {
        let runner = AgentLoopRunner::new(
            AgentLoopConfig::default(),
            "system".into(),
            vec![],
            HashMap::new(),
        );
        let llm = FakeLlm {
            response: "Hello!".into(),
            tool_calls: vec![],
        };
        let messages = vec![ChatMessage::new(
            MessageRole::User,
            "Hi".into(),
            MessageImportance::Normal,
            10,
        )];

        let result = runner
            .run(&llm, &SequentialToolExecutor, messages)
            .await
            .unwrap();
        assert_eq!(result.response, "Hello!");
        assert!(result.total_usage.is_some());
    }

    #[tokio::test]
    async fn runner_executes_tool_calls() {
        use std::sync::Arc;

        let search_handler: SkillHandler =
            Arc::new(|_args| Box::pin(async { Ok(serde_json::json!({"results": ["found it"]})) }));

        let mut handlers = HashMap::new();
        handlers.insert("search".to_string(), search_handler);

        let runner = AgentLoopRunner::new(
            AgentLoopConfig::default(),
            "system".into(),
            vec![serde_json::json!({"type": "function", "function": {"name": "search"}})],
            handlers,
        );

        let llm = FakeLlmWithTools {
            calls: std::sync::Mutex::new(0),
        };

        let messages = vec![ChatMessage::new(
            MessageRole::User,
            "Search for test".into(),
            MessageImportance::Normal,
            10,
        )];

        let result = runner
            .run(&llm, &SequentialToolExecutor, messages)
            .await
            .unwrap();
        assert_eq!(result.response, "Found the answer.");
        // Messages should contain: user + assistant(tool_calls) + tool_result + assistant(final)
        assert!(result.messages.len() >= 3);
    }

    #[tokio::test]
    async fn runner_respects_round_limit() {
        // LLM always returns tool calls — should be stopped by ToolRoundGuard.
        let always_tool = AlwaysToolLlm;
        // max_tool_rounds=2: round 0 OK, round 1 OK, round 2 guard fires (2>=2).
        let runner = AgentLoopRunner::new(
            AgentLoopConfig {
                max_tool_rounds: 2,
                ..Default::default()
            },
            "system".into(),
            vec![],
            HashMap::new(),
        );

        let messages = vec![ChatMessage::new(
            MessageRole::User,
            "go".into(),
            MessageImportance::Normal,
            10,
        )];

        let result = runner
            .run(&always_tool, &SequentialToolExecutor, messages)
            .await
            .unwrap();
        // Should have been aborted by ToolRoundGuardMiddleware.
        assert!(result.response.contains("maximum tool rounds"));
    }

    /// LLM that always returns a tool call — used to test round limiting.
    struct AlwaysToolLlm;

    #[async_trait]
    impl LlmBackend for AlwaysToolLlm {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: Option<&[serde_json::Value]>,
        ) -> anyhow::Result<LlmResult> {
            Ok(LlmResult {
                response: "calling tool...".into(),
                tool_calls: vec![ToolCallInfo {
                    id: format!("tc_{}", uuid::Uuid::new_v4()),
                    function_name: "unknown_tool".into(),
                    arguments: "{}".into(),
                }],
                usage: None,
            })
        }
    }
}
