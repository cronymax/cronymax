//! Agent loop engine — traits, runner, and built-in executors.
//!
//! This module provides the core agent loop abstraction:
//! - [`LlmBackend`] — abstracts streaming vs non-streaming LLM calls
//! - [`ToolExecutor`] — abstracts parallel vs sequential tool execution
//! - [`MemoryBackend`] — abstracts persistent vs in-memory recall
//! - [`LlmBackendFactory`] — creates model-specific LLM backends
//! - [`AgentLoopRunner`] — orchestrates the Context → Middleware → LLM → Tools loop
//! - [`SequentialToolExecutor`] / [`ParallelToolExecutor`] — built-in executors

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::middleware::{MiddlewareChain, MiddlewareChainConfig, MiddlewareContext};
use crate::types::{
    ChatMessage, MessageImportance, MessageRole, SkillHandler, TokenUsage, ToolCallInfo,
};

// ─── Configuration ───────────────────────────────────────────────────────────

/// Agent loop configuration — controls loop limits, context window, and delegation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentLoopConfig {
    /// Maximum tool-call rounds before aborting (default: 10).
    pub max_tool_rounds: usize,
    /// Maximum tokens for the full context window (default: 128_000).
    pub max_context_tokens: usize,
    /// Token ratio that triggers async compaction (default: 0.75).
    pub compaction_threshold_soft: f32,
    /// Token ratio that forces synchronous compaction (default: 0.90).
    pub compaction_threshold_hard: f32,
    /// Last N messages for sliding-window recency (default: 20).
    pub sliding_window_n: usize,
    /// Top-K semantically similar past messages for RAG (default: 5).
    pub rag_top_k: usize,
    /// Model used for context compaction (default: gpt-4o-mini).
    pub compaction_model: String,
    /// Maximum delegation depth for multi-agent orchestration (default: 3).
    pub max_delegation_depth: u32,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: 10,
            max_context_tokens: 128_000,
            compaction_threshold_soft: 0.75,
            compaction_threshold_hard: 0.90,
            sliding_window_n: 20,
            rag_top_k: 5,
            compaction_model: "gpt-4o-mini".into(),
            max_delegation_depth: 3,
        }
    }
}

// ─── Traits ──────────────────────────────────────────────────────────────────

/// LLM completion result.
#[derive(Debug, Clone)]
pub struct LlmResult {
    pub response: String,
    pub tool_calls: Vec<ToolCallInfo>,
    pub usage: Option<TokenUsage>,
}

/// Abstraction over LLM call strategies (streaming vs non-streaming).
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Send messages to the LLM and get a complete result.
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[serde_json::Value]>,
    ) -> anyhow::Result<LlmResult>;
}

/// Abstraction over tool execution strategies.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a batch of tool calls and return results keyed by tool_call_id.
    async fn execute_batch(
        &self,
        tool_calls: &[ToolCallInfo],
        handlers: &HashMap<String, SkillHandler>,
    ) -> Vec<(String, String)>;
}

/// Abstraction over memory backends (in-memory vs persistent).
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    /// Recall relevant context for the given query.
    async fn recall(&self, session_id: u32, query: &str) -> Vec<ChatMessage>;
    /// Save a message to memory.
    async fn save(&self, session_id: u32, message: &ChatMessage);
    /// Request compaction if supported (no-op for in-memory backends).
    async fn compact(&self, _session_id: u32, _candidates: &[ChatMessage]) {}
}

/// Factory for creating model-specific LLM backends.
///
/// Needed because sub-agents may use different models (cheap for triage,
/// powerful for code generation).
#[async_trait]
pub trait LlmBackendFactory: Send + Sync {
    /// Create an LLM backend configured for the given model.
    fn create(&self, model: &str) -> Box<dyn LlmBackend>;
}

// ─── Tool Executors ──────────────────────────────────────────────────────────

/// Sequential tool executor — runs each tool call one-by-one.
pub struct SequentialToolExecutor;

#[async_trait]
impl ToolExecutor for SequentialToolExecutor {
    async fn execute_batch(
        &self,
        tool_calls: &[ToolCallInfo],
        handlers: &HashMap<String, SkillHandler>,
    ) -> Vec<(String, String)> {
        let mut results = Vec::with_capacity(tool_calls.len());
        for tc in tool_calls {
            let result = if let Some(handler) = handlers.get(&tc.function_name) {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(serde_json::Value::Null);
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
            results.push((tc.id.clone(), result));
        }
        results
    }
}

/// Parallel tool executor — runs all tool calls concurrently via tokio::spawn.
pub struct ParallelToolExecutor;

#[async_trait]
impl ToolExecutor for ParallelToolExecutor {
    async fn execute_batch(
        &self,
        tool_calls: &[ToolCallInfo],
        handlers: &HashMap<String, SkillHandler>,
    ) -> Vec<(String, String)> {
        let mut join_handles = Vec::with_capacity(tool_calls.len());

        for tc in tool_calls {
            let tc_id = tc.id.clone();
            let func_name = tc.function_name.clone();
            let args_str = tc.arguments.clone();

            if let Some(handler) = handlers.get(&func_name) {
                let handler = handler.clone();
                let handle = tokio::spawn(async move {
                    let args: serde_json::Value =
                        serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
                    let result = match handler(args).await {
                        Ok(val) => serde_json::to_string(&val).unwrap_or_default(),
                        Err(e) => format!("Error executing {}: {}", func_name, e),
                    };
                    (tc_id, result)
                });
                join_handles.push(handle);
            } else {
                join_handles.push(tokio::spawn(async move {
                    (tc_id, format!("Unknown tool: {}", func_name))
                }));
            }
        }

        let mut results = Vec::with_capacity(join_handles.len());
        for handle in join_handles {
            match handle.await {
                Ok(pair) => results.push(pair),
                Err(e) => {
                    log::warn!("[AgentLoop] Tool join error: {}", e);
                }
            }
        }
        results
    }
}

// ─── Agent Loop Runner ───────────────────────────────────────────────────────

/// Result of a full agent loop execution.
pub struct AgentLoopResult {
    /// Final response text for the user.
    pub response: String,
    /// Complete message history including all tool call rounds.
    pub messages: Vec<ChatMessage>,
    /// The final assistant message (for memory saving).
    pub final_assistant: ChatMessage,
    /// Accumulated token usage across all rounds.
    pub total_usage: Option<TokenUsage>,
}

/// Unified agent loop runner that orchestrates the full pipeline.
///
/// This is the core loop: Context → Middleware(PRE) → LLM → Middleware(POST) → Tools → Loop
pub struct AgentLoopRunner {
    pub config: AgentLoopConfig,
    pub middleware: MiddlewareChain,
    pub system_prompt: String,
    pub tools: Vec<serde_json::Value>,
    pub skill_handlers: HashMap<String, SkillHandler>,
}

impl AgentLoopRunner {
    /// Create a new runner with the default middleware chain.
    pub fn new(
        config: AgentLoopConfig,
        system_prompt: String,
        tools: Vec<serde_json::Value>,
        skill_handlers: HashMap<String, SkillHandler>,
    ) -> Self {
        let middleware = MiddlewareChain::build_default(MiddlewareChainConfig::default());
        Self {
            config,
            middleware,
            system_prompt,
            tools,
            skill_handlers,
        }
    }

    /// Create a runner with a custom middleware chain.
    pub fn with_middleware(
        config: AgentLoopConfig,
        middleware: MiddlewareChain,
        system_prompt: String,
        tools: Vec<serde_json::Value>,
        skill_handlers: HashMap<String, SkillHandler>,
    ) -> Self {
        Self {
            config,
            middleware,
            system_prompt,
            tools,
            skill_handlers,
        }
    }

    /// Run the full agent loop pipeline for a set of context messages.
    pub async fn run(
        &self,
        llm: &dyn LlmBackend,
        tool_executor: &dyn ToolExecutor,
        mut messages: Vec<ChatMessage>,
    ) -> anyhow::Result<AgentLoopResult> {
        let tools_ref: Option<&[serde_json::Value]> = if self.tools.is_empty() {
            None
        } else {
            Some(&self.tools)
        };

        let mut response_text = String::new();
        let mut total_usage: Option<TokenUsage> = None;

        for round in 0..self.config.max_tool_rounds {
            // ── Middleware: before_llm ────────────────────────────────
            let total_tokens = messages.iter().map(|m| m.token_count as usize).sum();
            let mut mw_ctx = MiddlewareContext::new(
                round as u32,
                self.config.max_tool_rounds as u32,
                total_tokens,
                self.config.max_context_tokens,
            );
            let should_proceed = self.middleware.run_before_llm(&mut messages, &mut mw_ctx);
            if !should_proceed {
                response_text = mw_ctx
                    .abort_reason
                    .unwrap_or_else(|| "Agent loop stopped by middleware.".to_string());
                log::info!(
                    "[AgentLoop] Middleware aborted at round {}: {}",
                    round,
                    response_text
                );
                break;
            }

            // ── LLM call ─────────────────────────────────────────────
            let result = llm.complete(&messages, tools_ref).await?;

            // ── Middleware: after_llm ─────────────────────────────────
            let outcome =
                self.middleware
                    .run_after_llm(&result.response, &result.tool_calls, &mut mw_ctx);
            let effective_tool_calls = outcome.override_tool_calls.unwrap_or(result.tool_calls);
            let effective_response = outcome.override_response.unwrap_or(result.response);

            // Accumulate usage.
            if let Some(u) = result.usage {
                total_usage = Some(match total_usage {
                    Some(prev) => TokenUsage {
                        prompt_tokens: prev.prompt_tokens + u.prompt_tokens,
                        completion_tokens: prev.completion_tokens + u.completion_tokens,
                        total_tokens: prev.total_tokens + u.total_tokens,
                    },
                    None => u,
                });
            }

            // ── No tool calls → final response ──────────────────────
            if effective_tool_calls.is_empty() {
                response_text = effective_response;
                log::info!(
                    "[AgentLoop] LLM responded ({} chars, round {})",
                    response_text.len(),
                    round
                );
                break;
            }

            log::info!(
                "[AgentLoop] {} tool call(s) in round {}",
                effective_tool_calls.len(),
                round
            );

            // ── Add assistant message with tool calls ────────────────
            let mut assistant_msg = ChatMessage::new(
                MessageRole::Assistant,
                effective_response.clone(),
                MessageImportance::Normal,
                0,
            );
            assistant_msg.tool_calls = effective_tool_calls.clone();
            messages.push(assistant_msg);

            // ── Execute tools ────────────────────────────────────────
            let results = tool_executor
                .execute_batch(&effective_tool_calls, &self.skill_handlers)
                .await;

            for (tc_id, result) in results {
                let mut tool_msg =
                    ChatMessage::new(MessageRole::Tool, result, MessageImportance::Normal, 0);
                tool_msg.tool_call_id = Some(tc_id);
                messages.push(tool_msg);
            }
        }

        if response_text.is_empty() {
            response_text = format!(
                "Agent loop reached maximum tool rounds ({}/{})",
                self.config.max_tool_rounds, self.config.max_tool_rounds
            );
            log::warn!("[AgentLoop] {}", response_text);
        }

        let final_assistant = ChatMessage::new(
            MessageRole::Assistant,
            response_text.clone(),
            MessageImportance::Normal,
            (response_text.chars().count().div_ceil(4)) as u32,
        );

        Ok(AgentLoopResult {
            response: response_text,
            messages,
            final_assistant,
            total_usage,
        })
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct EchoBackend;

    #[async_trait]
    impl LlmBackend for EchoBackend {
        async fn complete(
            &self,
            messages: &[ChatMessage],
            _tools: Option<&[serde_json::Value]>,
        ) -> anyhow::Result<LlmResult> {
            let last = messages.last().map(|m| m.content.as_str()).unwrap_or("");
            Ok(LlmResult {
                response: format!("Echo: {}", last),
                tool_calls: vec![],
                usage: Some(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                }),
            })
        }
    }

    struct ToolCallingBackend {
        calls_remaining: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl LlmBackend for ToolCallingBackend {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: Option<&[serde_json::Value]>,
        ) -> anyhow::Result<LlmResult> {
            let remaining = self
                .calls_remaining
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if remaining > 0 {
                Ok(LlmResult {
                    response: String::new(),
                    tool_calls: vec![ToolCallInfo {
                        id: format!("call_{}", remaining),
                        function_name: "test_tool".into(),
                        arguments: "{}".into(),
                    }],
                    usage: None,
                })
            } else {
                Ok(LlmResult {
                    response: "Done with tools".into(),
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
            "You are helpful.".into(),
            vec![],
            HashMap::new(),
        );
        let messages = vec![ChatMessage::new(
            MessageRole::User,
            "hi".into(),
            MessageImportance::Normal,
            1,
        )];
        let result = runner
            .run(&EchoBackend, &SequentialToolExecutor, messages)
            .await
            .unwrap();
        assert!(result.response.contains("Echo:"));
        assert!(result.total_usage.is_some());
    }

    #[tokio::test]
    async fn runner_executes_tool_calls() {
        let handler: SkillHandler =
            Arc::new(|_args| Box::pin(async { Ok(serde_json::json!({"ok": true})) }));
        let mut handlers = HashMap::new();
        handlers.insert("test_tool".to_string(), handler);

        let runner = AgentLoopRunner::new(
            AgentLoopConfig::default(),
            "You are helpful.".into(),
            vec![serde_json::json!({"type": "function", "function": {"name": "test_tool"}})],
            handlers,
        );

        let backend = ToolCallingBackend {
            calls_remaining: std::sync::atomic::AtomicUsize::new(2),
        };

        let messages = vec![ChatMessage::new(
            MessageRole::User,
            "use tools".into(),
            MessageImportance::Normal,
            1,
        )];
        let result = runner
            .run(&backend, &SequentialToolExecutor, messages)
            .await
            .unwrap();
        assert_eq!(result.response, "Done with tools");
    }

    #[tokio::test]
    async fn runner_respects_round_limit() {
        let mut config = AgentLoopConfig::default();
        config.max_tool_rounds = 2;

        let handler: SkillHandler =
            Arc::new(|_args| Box::pin(async { Ok(serde_json::json!({"ok": true})) }));
        let mut handlers = HashMap::new();
        handlers.insert("test_tool".to_string(), handler);

        let runner = AgentLoopRunner::new(
            config,
            "You are helpful.".into(),
            vec![serde_json::json!({"type": "function", "function": {"name": "test_tool"}})],
            handlers,
        );

        // Backend that always returns tool calls
        let backend = ToolCallingBackend {
            calls_remaining: std::sync::atomic::AtomicUsize::new(100),
        };

        let messages = vec![ChatMessage::new(
            MessageRole::User,
            "spam tools".into(),
            MessageImportance::Normal,
            1,
        )];
        let result = runner
            .run(&backend, &SequentialToolExecutor, messages)
            .await
            .unwrap();
        assert!(result.response.contains("maximum tool rounds"));
    }
}
