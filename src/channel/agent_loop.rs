//! 6-stage agent loop pipeline for channel messages.
//!
//! Pipeline: Message IN → Memory Recall (CTX) → LLM (AI) → Tools/Skills (EXEC)
//!         → Memory Save / Context Compaction (STORE) → Response OUT.
#![allow(dead_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

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
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::context::{ChatMessage, MessageImportance, MessageRole};
use crate::ai::skills::SkillHandler;
use crate::ai::stream::{TokenUsage, ToolCallInfo};
use crate::channel::ChannelMessage;
use crate::channel::memory::ChannelMemoryStore;

// ─── Configuration ───────────────────────────────────────────────────────────

/// Agent loop configuration controlling memory recall and compaction behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentLoopConfig {
    /// Last N messages for sliding-window recency (default: 20).
    pub sliding_window_n: usize,
    /// Top-K semantically similar past messages for RAG (default: 5).
    pub rag_top_k: usize,
    /// Token ratio that triggers async compaction (default: 0.75).
    pub compaction_threshold_soft: f32,
    /// Token ratio that forces synchronous compaction (default: 0.90).
    pub compaction_threshold_hard: f32,
    /// Model used for context compaction (default: gpt-4o-mini).
    pub compaction_model: String,
    /// Maximum tokens for the full context window.
    pub max_context_tokens: usize,
    /// Maximum tool-call rounds before aborting (default: 10).
    pub max_tool_rounds: usize,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            sliding_window_n: 20,
            rag_top_k: 5,
            compaction_threshold_soft: 0.75,
            compaction_threshold_hard: 0.90,
            compaction_model: "gpt-4o-mini".into(),
            max_context_tokens: 128_000,
            max_tool_rounds: 10,
        }
    }
}

// ─── Runtime Dependencies ────────────────────────────────────────────────────

/// Runtime dependencies needed by the agent loop, constructed by the caller.
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

// ─── Stage 1: Message IN ─────────────────────────────────────────────────────

/// Normalize an incoming `ChannelMessage` into a `ChatMessage` for the LLM context.
pub fn normalize_message(msg: &ChannelMessage) -> ChatMessage {
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

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Derive a stable numeric session ID from a channel's (channel_id, chat_id) pair.
pub fn session_id_for_channel(channel_id: &str, chat_id: &str) -> u32 {
    let mut hasher = std::hash::DefaultHasher::new();
    channel_id.hash(&mut hasher);
    chat_id.hash(&mut hasher);
    // Keep in the high range to avoid colliding with terminal session IDs.
    900_000 + (hasher.finish() % 100_000) as u32
}

// ─── LLM Call ────────────────────────────────────────────────────────────────

/// Make a non-streaming LLM chat completion request and return the full result.
///
/// Uses the async-openai non-streaming API to collect a complete response.
/// Returns `(response_text, tool_calls, token_usage)`.
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

// ─── Pipeline Orchestration ──────────────────────────────────────────────────

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
pub async fn process_message(
    message: &ChannelMessage,
    config: &AgentLoopConfig,
    memory: &ChannelMemoryStore,
    deps: &AgentLoopDeps,
) -> anyhow::Result<String> {
    let session_id = session_id_for_channel(&message.channel_id, &message.chat_id);

    // ── Stage 1: Message IN ──────────────────────────────────────────────
    let user_msg = normalize_message(message);
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

    // ── Stage 3 & 4: LLM + Tool Execution Loop ─────────────────────────
    let tools_ref: Option<&[Value]> = if deps.tools.is_empty() {
        None
    } else {
        Some(&deps.tools)
    };

    let mut response_text = String::new();
    let mut total_usage: Option<TokenUsage> = None;

    for round in 0..config.max_tool_rounds {
        let (response, tool_calls, usage) =
            complete_chat(&deps.openai_client, &deps.model, &messages, tools_ref).await?;

        // Accumulate token usage.
        if let Some(u) = usage {
            total_usage = Some(match total_usage {
                Some(prev) => TokenUsage {
                    prompt_tokens: prev.prompt_tokens + u.prompt_tokens,
                    completion_tokens: prev.completion_tokens + u.completion_tokens,
                    total_tokens: prev.total_tokens + u.total_tokens,
                },
                None => u,
            });
        }

        if tool_calls.is_empty() {
            // No tool calls — final response.
            response_text = response;
            log::info!(
                "[AgentLoop] Stage 3: LLM responded ({} chars, round {})",
                response_text.len(),
                round
            );
            break;
        }

        log::info!(
            "[AgentLoop] Stage 4: {} tool call(s) in round {}",
            tool_calls.len(),
            round
        );

        // Add the assistant message with tool calls to context.
        let mut assistant_msg = ChatMessage::new(
            MessageRole::Assistant,
            response.clone(),
            MessageImportance::Normal,
            0,
        );
        assistant_msg.tool_calls = tool_calls.clone();
        messages.push(assistant_msg);

        // Execute each tool call.
        for tc in &tool_calls {
            let result = if let Some(handler) = deps.skill_handlers.get(&tc.function_name) {
                let args: Value = serde_json::from_str(&tc.arguments).unwrap_or(Value::Null);
                match handler(args).await {
                    Ok(val) => serde_json::to_string(&val).unwrap_or_default(),
                    Err(e) => {
                        log::warn!("[AgentLoop] Tool {} failed: {}", tc.function_name, e);
                        format!("Error executing {}: {}", tc.function_name, e)
                    }
                }
            } else {
                log::warn!("[AgentLoop] Unknown tool: {}", tc.function_name);
                format!("Unknown tool: {}", tc.function_name)
            };

            let mut tool_msg =
                ChatMessage::new(MessageRole::Tool, result, MessageImportance::Normal, 0);
            tool_msg.tool_call_id = Some(tc.id.clone());
            messages.push(tool_msg);
        }

        // If this is the last round, force a response by making the next
        // call without tools.
        if round == config.max_tool_rounds - 2 {
            log::warn!("[AgentLoop] Approaching tool call limit, next round will have no tools");
        }
    }

    // ── Stage 5: Memory Save / Context Compaction ───────────────────────
    // Save the user message and assistant response to memory.
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
        // Use all non-system messages as compaction candidates.
        let candidates: Vec<ChatMessage> = messages
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
