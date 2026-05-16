//! ReAct loop driver. See module-level docs in
//! [`crate::agent_loop`] for the high-level flow.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::StreamExt;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::llm::{
    ChatMessage, ChatRole, FinishReason, LlmEvent, LlmProvider, LlmRequest, ThinkingConfig,
    ToolCall,
};
use crate::protocol::events::RuntimeEventPayload;
use crate::runtime::authority::{AuthorityError, ReviewResolution, RuntimeAuthority};
use crate::runtime::middleware::{MiddlewareChain, TokenUsage, ToolGate, TurnContext};
use crate::runtime::state::{PermissionState, RunId, RunStatus, SessionId};

use super::tools::{ToolDispatcher, ToolOutcome};

/// Reasons the loop can fail. `MaxTurnsExceeded` is its own variant
/// so the host can surface it distinctly from arbitrary provider
/// errors.
#[derive(Debug, Error)]
pub enum LoopError {
    #[error("authority error: {0}")]
    Authority(#[from] AuthorityError),
    #[error("provider error: {0}")]
    Provider(#[source] anyhow::Error),
    #[error("loop exceeded max turns ({0})")]
    MaxTurnsExceeded(usize),
    #[error("review resolution channel closed before answer")]
    ReviewChannelClosed,
    #[error("loop cancelled")]
    Cancelled,
}

/// Reflection trigger condition.
#[derive(Clone, Debug)]
pub enum ReflectionTrigger {
    /// Fire every N completed turns.
    EveryNTurns(usize),
    /// Fire after N consecutive tool failures in a row.
    OnConsecutiveFailures(usize),
    /// Fire when either condition is met.
    Both {
        every_n_turns: usize,
        on_consecutive_failures: usize,
    },
}

/// Configuration for the in-loop reflection pass. When present on a
/// `LoopConfig`, the `ReactLoop` will fire a self-assessment prompt
/// at the configured trigger and append a `[REFLECTION]` sentinel
/// message to the history.
#[derive(Clone, Debug)]
pub struct ReflectionConfig {
    pub trigger: ReflectionTrigger,
    /// Override for the reflection prompt template. When `None`, the
    /// loop uses the default template embedded in the Crony builtin.
    pub prompt_template: Option<String>,
    /// Set to `false` to disable reflection without removing the config.
    pub enabled: bool,
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            trigger: ReflectionTrigger::EveryNTurns(4),
            prompt_template: None,
            enabled: true,
        }
    }
}

/// Per-run loop configuration. Cheap to clone; the underlying
/// provider/dispatcher are `Arc`-internal.
#[derive(Clone)]
pub struct LoopConfig {
    pub model: String,
    pub system_prompt: Option<String>,
    pub user_input: String,
    pub max_turns: usize,
    pub temperature: Option<f32>,
    /// OpenAI reasoning_effort (`minimal`/`low`/`medium`/`high`) forwarded
    /// on every LLM request in the loop. `None` = omit the field.
    pub reasoning_effort: Option<String>,
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<dyn ToolDispatcher>,
    /// Optional thinking/reasoning config. When `Some`, it is attached to
    /// every `LlmRequest` so the model emits thinking tokens.
    pub thinking: Option<ThinkingConfig>,
    /// Prior conversation thread from the session. When `Some` and
    /// non-empty, this replaces the default `[system, user]` history
    /// seed — the user message is appended to the end of this thread
    /// instead. When `None` or empty, the loop starts fresh.
    pub initial_thread: Option<Vec<ChatMessage>>,
    /// Session to flush the thread back to on completion.
    pub session_id: Option<SessionId>,
    /// Optional reflection configuration. When `Some`, the loop fires a
    /// self-assessment pass at the configured trigger and appends a
    /// `[REFLECTION]` sentinel message to the history.
    pub reflection: Option<ReflectionConfig>,
    /// Namespace to write reflection summaries and compaction summaries to.
    pub write_namespace: Option<crate::runtime::state::MemoryNamespaceId>,
    /// Memory manager shared with the runtime. `None` disables persistence-
    /// backed memory operations (write, search, summary).
    pub memory_manager: Option<Arc<crate::memory::MemoryManager>>,
    /// Middleware chain executed at each loop lifecycle point. Defaults to
    /// an empty (no-op) chain when not configured.
    pub middleware: Arc<MiddlewareChain>,
}

impl std::fmt::Debug for LoopConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopConfig")
            .field("model", &self.model)
            .field("system_prompt", &self.system_prompt)
            .field("user_input_len", &self.user_input.len())
            .field("max_turns", &self.max_turns)
            .field("temperature", &self.temperature)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("thinking", &self.thinking.as_ref().map(|_| "<config>"))
            .field("llm", &"<provider>")
            .field("tools", &"<dispatcher>")
            .field(
                "has_initial_thread",
                &self.initial_thread.as_ref().map(|t| t.len()),
            )
            .field(
                "session_id",
                &self.session_id.as_ref().map(|s| s.to_string()),
            )
            .field("reflection", &self.reflection.as_ref().map(|r| r.enabled))
            .field(
                "write_namespace",
                &self.write_namespace.as_ref().map(|n| n.0.as_str()),
            )
            .field("has_memory_manager", &self.memory_manager.is_some())
            .field("middleware_count", &self.middleware.0.len())
            .finish()
    }
}

/// Driver wired to a specific run. Not reusable across runs.
#[derive(Debug)]
pub struct ReactLoop {
    authority: RuntimeAuthority,
    run_id: RunId,
    config: LoopConfig,
    history: Vec<ChatMessage>,
    /// Monotonically increasing turn counter, also used as the
    /// `turn_id` field in `Token` events so the UI can group deltas.
    turn: u64,
    /// Number of consecutive tool invocation failures (reset on success).
    consecutive_failures: usize,
}

impl ReactLoop {
    pub fn new(authority: RuntimeAuthority, run_id: RunId, config: LoopConfig) -> Self {
        let history = if let Some(prior) = config.initial_thread.as_ref().filter(|t| !t.is_empty())
        {
            // Continue from a prior session thread: append the new user
            // message to the end of the existing conversation. The thread
            // already contains the system prompt from the first run, so we
            // don't add it again.
            let mut h = prior.clone();
            h.push(ChatMessage::user(config.user_input.clone()));
            h
        } else {
            // Fresh start: seed with system prompt (if any) + user message.
            let mut h = Vec::with_capacity(2);
            if let Some(sys) = &config.system_prompt {
                h.push(ChatMessage::system(sys.clone()));
            }
            h.push(ChatMessage::user(config.user_input.clone()));
            h
        };
        Self {
            authority,
            run_id,
            config,
            history,
            turn: 0,
            consecutive_failures: 0,
        }
    }

    /// Drive the loop to completion (success, failure, or cancellation).
    /// Errors flow through both the returned `Result` *and* a `Failed`
    /// run-status transition so subscribers see them.
    pub async fn run(mut self) -> Result<(), LoopError> {
        // Promote Pending -> Running. Subscribers see this as a
        // RunStatus event with status="running".
        self.authority.mark_run_running(self.run_id)?;

        // Emit run_start trace as the first event so the UI can display
        // what the agent received before any LLM call.
        let tool_names: Vec<String> = self
            .config
            .tools
            .definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();
        self.authority.emit_for_run(
            self.run_id,
            RuntimeEventPayload::Trace {
                run_id: self.run_id.to_string(),
                trace: serde_json::json!({
                    "kind": "run_start",
                    "model": self.config.model,
                    "system_prompt": self.config.system_prompt.as_deref().unwrap_or(""),
                    "user_input": self.config.user_input,
                    "tools": tool_names,
                    "turns_limit": self.config.max_turns,
                }),
            },
        );

        let outcome = self.drive().await;

        // Flush thread back to session (success or failure) so that the
        // next run in this session sees a continuous conversation.
        if let Some(ref sid) = self.config.session_id.clone() {
            let _ = self.authority.flush_thread(sid, self.history.clone());
        }

        match &outcome {
            Ok(()) => {
                // `complete_run` is idempotent if a Terminal tool
                // already finished the run.
                let _ = self.authority.complete_run(self.run_id);
            }
            Err(LoopError::Cancelled) => {
                // Caller already cancelled the run; don't overwrite.
            }
            Err(e) => {
                let _ = self.authority.fail_run(self.run_id, e.to_string());
            }
        }
        outcome
    }

    async fn drive(&mut self) -> Result<(), LoopError> {
        for _ in 0..self.config.max_turns {
            // Snapshot current run status; bail out if the host
            // cancelled or paused us.
            match self.authority.run_status(self.run_id)? {
                RunStatus::Cancelled => return Err(LoopError::Cancelled),
                RunStatus::Paused => return Err(LoopError::Cancelled),
                RunStatus::Succeeded | RunStatus::Failed { .. } => return Ok(()),
                _ => {}
            }
            self.turn += 1;
            let turn_id = self.turn;

            // Construct TurnContext for this turn.
            // TokenAccumulatorMiddleware will fill in total_usage during before_llm_call.
            let mut ctx = TurnContext {
                run_id: self.run_id,
                turn: turn_id,
                model: self.config.model.clone(),
                total_usage: TokenUsage::default(),
            };

            // Call before_llm_call first so total_usage is set correctly.
            self.config.middleware.before_llm_call(&mut ctx).await;

            // ── Reflection trigger ────────────────────────────────────────
            if let Some(ref rcfg) = self.config.reflection.clone() {
                if rcfg.enabled {
                    let should_reflect = match &rcfg.trigger {
                        ReflectionTrigger::EveryNTurns(n) => *n > 0 && self.turn % (*n as u64) == 0,
                        ReflectionTrigger::OnConsecutiveFailures(n) => {
                            *n > 0 && self.consecutive_failures >= *n
                        }
                        ReflectionTrigger::Both {
                            every_n_turns,
                            on_consecutive_failures,
                        } => {
                            (*every_n_turns > 0 && self.turn % (*every_n_turns as u64) == 0)
                                || (*on_consecutive_failures > 0
                                    && self.consecutive_failures >= *on_consecutive_failures)
                        }
                    };
                    if should_reflect {
                        self.config.middleware.before_reflection(&ctx).await;
                        if let Some(refl_text) = self.run_reflection_pass(rcfg).await {
                            self.config
                                .middleware
                                .after_reflection(&ctx, &refl_text)
                                .await;
                        }
                    }
                }
            }

            let req = LlmRequest {
                model: self.config.model.clone(),
                messages: self.history.clone(),
                tools: self.config.tools.definitions(),
                temperature: self.config.temperature,
                reasoning_effort: self.config.reasoning_effort.clone(),
                thinking: self.config.thinking.clone(),
            };

            let mut stream = match self.config.llm.stream(req).await {
                Ok(s) => s,
                Err(e) => {
                    // reqwest hides the actual root cause (DNS error,
                    // connection timeout, TLS handshake, ...) inside
                    // anyhow's source chain — flatten it into one
                    // string so the UI gets actionable context.
                    let full = format_error_chain(&e);
                    info!(run = %self.run_id, error = %full, "llm stream failed");
                    self.authority.emit_for_run(
                        self.run_id,
                        RuntimeEventPayload::Trace {
                            run_id: self.run_id.to_string(),
                            trace: serde_json::json!({
                                "kind": "error",
                                "where": "llm.stream",
                                "message": full,
                            }),
                        },
                    );
                    return Err(LoopError::Provider(e));
                }
            };

            let mut text = String::new();
            let mut thinking_buf = String::new(); // accumulated thinking; NOT added to history
            let mut calls: BTreeMap<usize, AccumCall> = BTreeMap::new();
            let mut finish: Option<FinishReason> = None;
            let mut turn_usage = TokenUsage::default();

            while let Some(event) = stream.next().await {
                match event {
                    LlmEvent::ThinkingDelta { content } => {
                        thinking_buf.push_str(&content);
                        self.authority.emit_for_run(
                            self.run_id,
                            RuntimeEventPayload::ThinkingToken {
                                run_id: self.run_id.to_string(),
                                turn_id: turn_id.to_string(),
                                delta: content,
                            },
                        );
                    }
                    LlmEvent::Delta { content } => {
                        text.push_str(&content);
                        self.authority.emit_for_run(
                            self.run_id,
                            RuntimeEventPayload::Token {
                                run_id: self.run_id.to_string(),
                                turn_id: turn_id.to_string(),
                                delta: content,
                            },
                        );
                    }
                    LlmEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_chunk,
                    } => {
                        let entry = calls.entry(index).or_default();
                        // Only overwrite if the provider sent a
                        // non-empty value — some compat APIs spam
                        // `name: ""` deltas that would otherwise wipe
                        // a previously-set name.
                        if let Some(id) = id {
                            if !id.is_empty() {
                                entry.id = id;
                            }
                        }
                        if let Some(name) = name {
                            if !name.is_empty() {
                                entry.name = name;
                            }
                        }
                        if let Some(chunk) = arguments_chunk {
                            entry.arguments.push_str(&chunk);
                        }
                    }
                    LlmEvent::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        turn_usage += TokenUsage {
                            input_tokens,
                            output_tokens,
                        };
                    }
                    LlmEvent::Done { finish_reason } => {
                        finish = Some(finish_reason);
                        break;
                    }
                    LlmEvent::Error { message } => {
                        self.authority.emit_for_run(
                            self.run_id,
                            RuntimeEventPayload::Trace {
                                run_id: self.run_id.to_string(),
                                trace: serde_json::json!({
                                    "kind": "error",
                                    "where": "llm.stream.event",
                                    "message": &message,
                                }),
                            },
                        );
                        return Err(LoopError::Provider(anyhow::anyhow!(message)));
                    }
                }
            }

            let finish = finish.unwrap_or(FinishReason::Stop);
            let tool_calls: Vec<ToolCall> = calls
                .into_values()
                .map(|c| ToolCall {
                    id: c.id,
                    name: c.name,
                    arguments: c.arguments,
                })
                .collect();

            // Record assistant turn in history (text and/or tool calls).
            if !tool_calls.is_empty() {
                self.history
                    .push(ChatMessage::assistant_tool_calls(tool_calls.clone()));
            } else if !text.is_empty() {
                self.history.push(ChatMessage::assistant_text(text.clone()));
            } else {
                // Empty assistant turn: still recorded so the next
                // request includes a placeholder.
                self.history.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: Some(String::new()),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    name: None,
                });
            }
            // Notify middleware (TraceEmitterMiddleware emits the assistant_turn trace).
            self.config
                .middleware
                .after_llm_call(&ctx, &text, &finish, &turn_usage)
                .await;

            match finish {
                FinishReason::Stop | FinishReason::Length | FinishReason::Other(_) => {
                    debug!(run = %self.run_id, "loop terminating: finish_reason stop/length/other");
                    return Ok(());
                }
                FinishReason::ToolCalls => {
                    if tool_calls.is_empty() {
                        warn!(
                            run = %self.run_id,
                            "finish_reason=tool_calls but no calls accumulated; terminating"
                        );
                        return Ok(());
                    }
                    // Dump everything we accumulated so the UI/logs can
                    // show what arrived from the provider. Catches
                    // cases where a non-standard streaming format
                    // produced empty names or ids.
                    self.authority.emit_for_run(
                        self.run_id,
                        RuntimeEventPayload::Trace {
                            run_id: self.run_id.to_string(),
                            trace: serde_json::json!({
                                "kind": "tool_calls_parsed",
                                "turn": turn_id,
                                "calls": tool_calls.iter().map(|c| serde_json::json!({
                                    "id": c.id,
                                    "name": c.name,
                                    "args_len": c.arguments.len(),
                                    "args_preview": c.arguments.chars().take(200).collect::<String>(),
                                })).collect::<Vec<_>>(),
                            }),
                        },
                    );
                    // Bail early on malformed calls — otherwise we'd
                    // dispatch with an empty name, get back
                    // "no tool registered: ", feed that to the model,
                    // and the model keeps retrying until max_turns.
                    if let Some(bad) = tool_calls.iter().find(|c| c.name.is_empty()) {
                        let msg = format!(
                            "provider returned a tool_call with empty name (id={:?}, args_len={}). \
                             This usually means the OpenAI-compatible endpoint streams tool_calls \
                             in a non-standard format (e.g. via `message` instead of `delta`, or \
                             omits `function.name`). The chat cannot proceed.",
                            bad.id, bad.arguments.len()
                        );
                        self.authority.emit_for_run(
                            self.run_id,
                            RuntimeEventPayload::Trace {
                                run_id: self.run_id.to_string(),
                                trace: serde_json::json!({
                                    "kind": "error",
                                    "where": "react.tool_calls",
                                    "message": &msg,
                                }),
                            },
                        );
                        return Err(LoopError::Provider(anyhow::anyhow!(msg)));
                    }
                    let mut terminal_seen = false;
                    for call in &tool_calls {
                        match self.run_one_tool(call, &ctx).await? {
                            ToolStepResult::Continue => {}
                            ToolStepResult::Terminal => {
                                terminal_seen = true;
                            }
                            ToolStepResult::Cancelled => {
                                return Err(LoopError::Cancelled);
                            }
                        }
                    }
                    if terminal_seen {
                        info!(run = %self.run_id, "terminal tool completed; loop done");
                        return Ok(());
                    }
                }
            }
        }
        Err(LoopError::MaxTurnsExceeded(self.config.max_turns))
    }

    async fn run_one_tool(
        &mut self,
        call: &ToolCall,
        ctx: &TurnContext,
    ) -> Result<ToolStepResult, LoopError> {
        // Gate: middleware can block tool dispatch (e.g. rate limiting, cost cap).
        // TraceEmitterMiddleware also emits `tool_start` in before_tool_call.
        match self.config.middleware.before_tool_call(ctx, call).await {
            ToolGate::Allow => {}
            ToolGate::Block(reason) => {
                let block_result = serde_json::json!({"error": reason.clone()});
                let serialized = serde_json::to_string(&block_result)
                    .unwrap_or_else(|_| format!("{{\"error\":\"{reason}\"}}"));
                self.history.push(ChatMessage::tool_result(
                    call.id.clone(),
                    call.name.clone(),
                    serialized,
                ));
                self.config
                    .middleware
                    .after_tool_call(ctx, call, &ToolOutcome::Error(reason))
                    .await;
                return Ok(ToolStepResult::Continue);
            }
        }

        let mut outcome = self.config.tools.dispatch(call).await;

        if let ToolOutcome::NeedsApproval { request } = outcome {
            // Open a review; this transitions the run to AwaitingReview
            // and emits the PermissionRequest event.
            let handle = self.authority.open_review_with_completion(
                self.run_id,
                serde_json::json!({
                    "kind": "tool_call",
                    "tool": call.name,
                    "tool_call_id": call.id,
                    "arguments": call.arguments,
                    "request": request,
                }),
            )?;
            let resolution = handle
                .completion
                .await
                .map_err(|_| LoopError::ReviewChannelClosed)?;
            match resolution.decision {
                PermissionState::Approved => {
                    outcome = self.config.tools.dispatch_approved(call).await;
                }
                PermissionState::Rejected => {
                    let msg = resolution
                        .notes
                        .clone()
                        .unwrap_or_else(|| "user rejected".into());
                    outcome = ToolOutcome::Error(format!("permission denied: {msg}"));
                }
                PermissionState::Deferred => {
                    // Treat deferred as "park the run" — caller should
                    // resume later via post_input + resolve_review.
                    return Err(LoopError::Cancelled);
                }
                PermissionState::Pending => {
                    return Err(LoopError::ReviewChannelClosed);
                }
            }
        }

        // Notify middleware before consuming the outcome (TraceEmitterMiddleware emits `tool_done`).
        self.config
            .middleware
            .after_tool_call(ctx, call, &outcome)
            .await;

        let (result_value, terminal) = match outcome {
            ToolOutcome::Output(v) => {
                self.consecutive_failures = 0;
                (v, false)
            }
            ToolOutcome::Error(e) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                (serde_json::json!({"error": e}), false)
            }
            ToolOutcome::Terminal(v) => {
                self.consecutive_failures = 0;
                (v, true)
            }
            ToolOutcome::NeedsApproval { .. } => {
                // dispatch_approved should never re-return NeedsApproval.
                (
                    serde_json::json!({"error": "tool repeatedly needs approval"}),
                    false,
                )
            }
        };

        let serialized = match serde_json::to_string(&result_value) {
            Ok(s) => s,
            Err(e) => format!("{{\"error\":\"serialize tool result: {e}\"}}"),
        };
        self.history.push(ChatMessage::tool_result(
            call.id.clone(),
            call.name.clone(),
            serialized,
        ));
        if terminal {
            Ok(ToolStepResult::Terminal)
        } else {
            Ok(ToolStepResult::Continue)
        }
    }

    /// Fire a reflection pass: call the LLM with the reflection prompt plus a
    /// truncated recent history, append the result as a `[REFLECTION]` system
    /// message, and optionally persist it to the write namespace.
    ///
    /// Returns the reflection text so the caller can invoke middleware hooks.
    /// Returns `None` on failure or if the LLM produced no output.
    /// This is best-effort: failures are logged but do not abort the loop.
    async fn run_reflection_pass(&mut self, cfg: &ReflectionConfig) -> Option<String> {
        // Build the reflection prompt from the template or default.
        let template = cfg
            .prompt_template
            .as_deref()
            .unwrap_or(crate::crony::prompts::REFLECT_PROMPT);

        // Include the last 6 messages as context (truncate to keep token use low).
        let context_messages: Vec<_> = self.history.iter().rev().take(6).rev().cloned().collect();

        let mut reflect_history = context_messages;
        reflect_history.push(ChatMessage::user(template.to_owned()));

        let req = LlmRequest {
            model: self.config.model.clone(),
            messages: reflect_history,
            tools: vec![], // No tools during reflection.
            temperature: Some(0.3),
            reasoning_effort: None,
            thinking: None,
        };

        let text = match self.config.llm.stream(req).await {
            Ok(mut stream) => {
                let mut buf = String::new();
                while let Some(event) = stream.next().await {
                    match event {
                        LlmEvent::Delta { content } => buf.push_str(&content),
                        LlmEvent::Done { .. } => break,
                        LlmEvent::Error { message } => {
                            warn!(run = %self.run_id, error = %message, "reflection_pass: llm error");
                            return None;
                        }
                        _ => {}
                    }
                }
                buf
            }
            Err(e) => {
                warn!(run = %self.run_id, error = %e, "reflection_pass: stream failed");
                return None;
            }
        };

        if text.is_empty() {
            return None;
        }

        let sentinel = format!("[REFLECTION] {text}");
        info!(run = %self.run_id, turn = self.turn, "reflection_pass: appended to history");

        // Append as a system message so the LLM sees it as context, not output.
        self.history.push(ChatMessage::system(sentinel.clone()));

        // Persist to write namespace if configured.
        if let (Some(ns), Some(mgr)) = (&self.config.write_namespace, &self.config.memory_manager) {
            let key = format!("reflect/turn_{}", self.turn);
            let _ = mgr.write(&ns.0, key, sentinel).await;
        }

        // Reset consecutive failure counter after reflection.
        self.consecutive_failures = 0;

        Some(text)
    }
}

#[derive(Default, Debug)]
struct AccumCall {
    id: String,
    name: String,
    arguments: String,
}

/// Walk an `anyhow::Error`'s source chain and join every layer with
/// `: `. reqwest wraps things deeply (e.g. `error sending request →
/// connection error → tcp connect error → operation timed out`); we
/// want the user to see the root cause too.
fn format_error_chain(e: &anyhow::Error) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let msg = s.to_string();
        if !out.contains(&msg) {
            out.push_str(": ");
            out.push_str(&msg);
        }
        src = s.source();
    }
    out
}

#[allow(dead_code)]
enum ToolStepResult {
    Continue,
    Terminal,
    Cancelled,
}

// Re-export for callers building loops directly.
pub use crate::runtime::authority::ReviewHandle;

#[allow(dead_code)]
fn _assert_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<ReactLoop>();
    check::<ReviewResolution>();
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 5.4: focused tests for runtime-owned LLM streaming, tool-call loops,
// and human-approval pauses.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use uuid::Uuid;

    use super::*;
    use crate::agent_loop::tools::{ToolDispatcher, ToolOutcome};
    use crate::llm::{FinishReason, MockLlmProvider, MockScript, ToolCall, ToolDef};
    use crate::protocol::events::RuntimeEventPayload;
    use crate::runtime::middleware::MiddlewareChain;
    use crate::runtime::state::{PermissionState, ReviewId, Space, SpaceId};
    use crate::runtime::SubscribeOutcome;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_authority() -> (RuntimeAuthority, SpaceId) {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "test".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let sid = space.id;
        auth.upsert_space(space).unwrap();
        (auth, sid)
    }

    fn make_config(
        llm: Arc<dyn crate::llm::LlmProvider>,
        tools: Arc<dyn ToolDispatcher>,
    ) -> LoopConfig {
        LoopConfig {
            model: "mock".into(),
            system_prompt: None,
            user_input: "hello".into(),
            max_turns: 10,
            temperature: None,
            reasoning_effort: None,
            llm,
            tools,
            thinking: None,
            initial_thread: None,
            session_id: None,
            reflection: None,
            write_namespace: None,
            memory_manager: None,
            middleware: Arc::new(MiddlewareChain::empty()),
        }
    }

    // ── scripted dispatcher: echoes arguments ─────────────────────────────────

    #[derive(Clone, Debug, Default)]
    struct EchoDispatcher;

    #[async_trait]
    impl ToolDispatcher for EchoDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "echo".into(),
                description: "echoes its arguments".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            }]
        }

        async fn dispatch(&self, call: &ToolCall) -> ToolOutcome {
            ToolOutcome::Output(serde_json::json!({"echoed": call.arguments}))
        }
    }

    // ── scripted dispatcher: first call NeedsApproval, approved re-dispatch ──

    #[derive(Clone, Debug, Default)]
    struct ApprovalDispatcher;

    #[async_trait]
    impl ToolDispatcher for ApprovalDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "needs_approval".into(),
                description: "requires a review".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            }]
        }

        async fn dispatch(&self, _call: &ToolCall) -> ToolOutcome {
            ToolOutcome::NeedsApproval {
                request: serde_json::json!({"what": "shell command"}),
            }
        }

        async fn dispatch_approved(&self, call: &ToolCall) -> ToolOutcome {
            ToolOutcome::Output(serde_json::json!({"approved": call.name}))
        }
    }

    // ── scripted dispatcher: terminal tool ────────────────────────────────────

    #[derive(Clone, Debug, Default)]
    struct TerminalDispatcher;

    #[async_trait]
    impl ToolDispatcher for TerminalDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "submit".into(),
                description: "ends the run".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            }]
        }

        async fn dispatch(&self, _call: &ToolCall) -> ToolOutcome {
            ToolOutcome::Terminal(serde_json::json!({"submitted": true}))
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    /// A plain-text response with no tool calls completes the run as Succeeded.
    #[tokio::test]
    async fn text_only_run_succeeds() {
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        llm.push(
            MockScript::new()
                .delta("Hello, ")
                .delta("world!")
                .done(FinishReason::Stop),
        );
        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        let cfg = make_config(llm.clone(), Arc::new(EchoDispatcher));
        ReactLoop::new(auth.clone(), run_id, cfg)
            .run()
            .await
            .unwrap();

        assert!(auth.run_status(run_id).unwrap().is_terminal());
        // Exactly one LLM turn (user message → stop)
        assert_eq!(llm.requests().len(), 1);
    }

    /// Token deltas emitted during a run surface as `Token` subscription events.
    #[tokio::test]
    async fn token_events_emitted_during_streaming() {
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        llm.push(
            MockScript::new()
                .delta("chunk-a")
                .delta("chunk-b")
                .done(FinishReason::Stop),
        );
        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        // Subscribe before the loop runs.
        let SubscribeOutcome {
            id: _,
            mut receiver,
        } = auth.subscribe(format!("run:{run_id}"));
        let cfg = make_config(llm.clone(), Arc::new(EchoDispatcher));
        ReactLoop::new(auth.clone(), run_id, cfg)
            .run()
            .await
            .unwrap();
        // Drain all events and count Token ones.
        let mut token_deltas: Vec<String> = Vec::new();
        while let Ok(ev) = receiver.try_recv() {
            if let RuntimeEventPayload::Token { delta, .. } = ev.payload {
                token_deltas.push(delta);
            }
        }
        assert_eq!(token_deltas, vec!["chunk-a", "chunk-b"]);
    }

    /// A tool call is dispatched, its result appended, then a second LLM
    /// turn produces a Stop — run ends as Succeeded.
    #[tokio::test]
    async fn tool_call_dispatched_then_stop_succeeds() {
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        // Turn 1: one tool call
        llm.push(
            MockScript::new()
                .tool_call(0, "call-1", "echo", r#"{"arg":"hi"}"#)
                .done(FinishReason::ToolCalls),
        );
        // Turn 2: stop after seeing tool result
        llm.push(MockScript::new().delta("Done.").done(FinishReason::Stop));

        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        let cfg = make_config(llm.clone(), Arc::new(EchoDispatcher));
        ReactLoop::new(auth.clone(), run_id, cfg)
            .run()
            .await
            .unwrap();

        assert!(auth.run_status(run_id).unwrap().is_terminal());
        let reqs = llm.requests();
        // Two LLM calls: initial + after tool result
        assert_eq!(reqs.len(), 2);
        // Second request must contain a tool-result message
        let last = &reqs[1];
        assert!(
            last.messages.iter().any(|m| m.tool_call_id.is_some()),
            "second request should contain a tool result message"
        );
    }

    /// A terminal tool ends the loop without a second LLM turn.
    #[tokio::test]
    async fn terminal_tool_ends_loop_immediately() {
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        llm.push(
            MockScript::new()
                .tool_call(0, "call-t", "submit", r#"{}"#)
                .done(FinishReason::ToolCalls),
        );
        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        let cfg = make_config(llm.clone(), Arc::new(TerminalDispatcher));
        ReactLoop::new(auth.clone(), run_id, cfg)
            .run()
            .await
            .unwrap();

        assert!(auth.run_status(run_id).unwrap().is_terminal());
        // Only one LLM turn — terminal tool fires before a second call
        assert_eq!(llm.requests().len(), 1);
    }

    /// Human-approval pause: loop opens a review, run transitions to
    /// AwaitingReview, then `resolve_review(Approved)` fires the completion
    /// oneshot and the loop continues to a successful finish.
    #[tokio::test]
    async fn approval_pause_then_resume_completes() {
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        // Turn 1: tool that needs approval
        llm.push(
            MockScript::new()
                .tool_call(0, "call-rev", "needs_approval", r#"{}"#)
                .done(FinishReason::ToolCalls),
        );
        // Turn 2: after approval the loop continues and stops
        llm.push(
            MockScript::new()
                .delta("Approved and done.")
                .done(FinishReason::Stop),
        );

        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        // Subscribe to watch for PermissionRequest events.
        let SubscribeOutcome {
            id: _,
            mut receiver,
        } = auth.subscribe(format!("run:{run_id}"));

        let auth2 = auth.clone();
        let cfg = make_config(llm.clone(), Arc::new(ApprovalDispatcher));
        let loop_task = tokio::spawn(async move { ReactLoop::new(auth2, run_id, cfg).run().await });

        // Wait until the run transitions to AwaitingReview (PermissionRequest event).
        let review_id: ReviewId = loop {
            let ev = receiver.recv().await.expect("subscription channel closed");
            match ev.payload {
                RuntimeEventPayload::PermissionRequest { review_id, .. } => {
                    break ReviewId(Uuid::parse_str(&review_id).expect("valid uuid"));
                }
                _ => {}
            }
        };

        // Approve the review — fires the completion oneshot inside the loop.
        auth.resolve_review(run_id, review_id, PermissionState::Approved, None)
            .unwrap();

        // Loop should complete with Ok(()).
        loop_task.await.unwrap().unwrap();
        assert!(auth.run_status(run_id).unwrap().is_terminal());
        // Two LLM turns: initial tool-call turn + post-approval turn
        assert_eq!(llm.requests().len(), 2);
    }

    /// Rejecting a review causes the tool to be reported as an error to the
    /// model (rather than crashing the loop) — the loop continues.
    #[tokio::test]
    async fn approval_rejected_reports_error_to_model() {
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        llm.push(
            MockScript::new()
                .tool_call(0, "call-rej", "needs_approval", r#"{}"#)
                .done(FinishReason::ToolCalls),
        );
        // After rejection the tool result is an error message; model stops.
        llm.push(
            MockScript::new()
                .delta("Understood.")
                .done(FinishReason::Stop),
        );

        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        let SubscribeOutcome {
            id: _,
            mut receiver,
        } = auth.subscribe(format!("run:{run_id}"));

        let auth2 = auth.clone();
        let cfg = make_config(llm.clone(), Arc::new(ApprovalDispatcher));
        let loop_task = tokio::spawn(async move { ReactLoop::new(auth2, run_id, cfg).run().await });

        let review_id: ReviewId = loop {
            let ev = receiver.recv().await.expect("subscription channel closed");
            match ev.payload {
                RuntimeEventPayload::PermissionRequest { review_id, .. } => {
                    break ReviewId(Uuid::parse_str(&review_id).expect("valid uuid"));
                }
                _ => {}
            }
        };

        auth.resolve_review(
            run_id,
            review_id,
            PermissionState::Rejected,
            Some("not allowed".into()),
        )
        .unwrap();

        loop_task.await.unwrap().unwrap();
        assert!(auth.run_status(run_id).unwrap().is_terminal());
        // Two turns: the loop continued after rejection
        assert_eq!(llm.requests().len(), 2);
        // The second request should contain a tool result carrying the rejection message.
        let reqs = llm.requests();
        let tool_msg = reqs[1]
            .messages
            .iter()
            .find(|m| m.tool_call_id.is_some())
            .expect("expected tool result message in second turn");
        let content = tool_msg.content.as_deref().unwrap_or("");
        assert!(content.contains("permission denied"), "content: {content}");
    }

    // ── 10.5: ReactLoop with mock LLM that emits Usage ────────────────────────

    /// A recording middleware that captures the `TokenUsage` passed to `after_llm_call`.
    struct UsageCapture {
        last_usage: parking_lot::Mutex<Option<crate::runtime::middleware::TokenUsage>>,
    }

    impl UsageCapture {
        fn new() -> Self {
            Self {
                last_usage: parking_lot::Mutex::new(None),
            }
        }
        fn last(&self) -> Option<crate::runtime::middleware::TokenUsage> {
            *self.last_usage.lock()
        }
    }

    #[async_trait]
    impl crate::runtime::middleware::RunMiddleware for UsageCapture {
        async fn after_llm_call(
            &self,
            _ctx: &crate::runtime::middleware::TurnContext,
            _text: &str,
            _finish: &FinishReason,
            usage: &crate::runtime::middleware::TokenUsage,
        ) {
            *self.last_usage.lock() = Some(*usage);
        }
    }

    #[tokio::test]
    async fn after_llm_call_receives_token_usage_from_mock_provider() {
        use crate::runtime::middleware::{MiddlewareChain, TokenUsage};
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        llm.push(
            MockScript::new()
                .usage(10, 5) // emit Usage before Done
                .delta("Hello!")
                .done(FinishReason::Stop),
        );
        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        let capture = Arc::new(UsageCapture::new());
        let mut cfg = make_config(llm.clone(), Arc::new(EchoDispatcher));
        cfg.middleware = Arc::new(MiddlewareChain(vec![capture.clone()]));
        ReactLoop::new(auth.clone(), run_id, cfg)
            .run()
            .await
            .unwrap();

        let usage = capture
            .last()
            .expect("after_llm_call should have been called");
        assert_eq!(
            usage,
            TokenUsage {
                input_tokens: 10,
                output_tokens: 5
            },
            "middleware should receive the usage emitted by mock"
        );
    }

    // ── 10.6: ToolGate::Block records error without dispatching tool ──────────

    /// A dispatcher that panics if dispatched — used to assert that a blocked
    /// tool is never actually dispatched.
    #[derive(Debug)]
    struct PanicDispatcher;

    #[async_trait]
    impl ToolDispatcher for PanicDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "forbidden".into(),
                description: "should never run".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            }]
        }
        async fn dispatch(&self, _call: &ToolCall) -> ToolOutcome {
            panic!("PanicDispatcher::dispatch called — tool should have been blocked");
        }
    }

    /// A middleware that blocks a specific tool by name.
    struct BlockToolMiddleware(String);

    #[async_trait]
    impl crate::runtime::middleware::RunMiddleware for BlockToolMiddleware {
        async fn before_tool_call(
            &self,
            _ctx: &crate::runtime::middleware::TurnContext,
            call: &ToolCall,
        ) -> crate::runtime::middleware::ToolGate {
            if call.name == self.0 {
                crate::runtime::middleware::ToolGate::Block("tool is forbidden".into())
            } else {
                crate::runtime::middleware::ToolGate::Allow
            }
        }
    }

    #[tokio::test]
    async fn blocked_tool_records_error_without_dispatching() {
        use crate::runtime::middleware::MiddlewareChain;
        let (auth, sid) = make_authority();
        let llm = Arc::new(MockLlmProvider::new());
        // Turn 1: call the forbidden tool
        llm.push(
            MockScript::new()
                .tool_call(0, "call-blocked", "forbidden", r#"{}"#)
                .done(FinishReason::ToolCalls),
        );
        // Turn 2: stop after seeing the (error) tool result
        llm.push(MockScript::new().delta("Noted.").done(FinishReason::Stop));

        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        let blocker = Arc::new(BlockToolMiddleware("forbidden".into()));
        let mut cfg = make_config(llm.clone(), Arc::new(PanicDispatcher));
        cfg.middleware = Arc::new(MiddlewareChain(vec![blocker]));
        ReactLoop::new(auth.clone(), run_id, cfg)
            .run()
            .await
            .unwrap();

        assert!(auth.run_status(run_id).unwrap().is_terminal());
        // Two turns: first calls the tool (blocked), second sees the error result and stops.
        assert_eq!(llm.requests().len(), 2);
        // The second request must contain a tool-result message with error content.
        let reqs = llm.requests();
        let tool_msg = reqs[1]
            .messages
            .iter()
            .find(|m| m.tool_call_id.is_some())
            .expect("expected tool result message in second turn");
        let content = tool_msg.content.as_deref().unwrap_or("");
        assert!(
            content.contains("tool is forbidden"),
            "tool result should contain block reason, got: {content}"
        );
    }
}
