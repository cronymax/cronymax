//! ReAct loop driver. See module-level docs in
//! [`crate::agent_loop`] for the high-level flow.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::StreamExt;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::llm::{
    ChatMessage, ChatRole, FinishReason, LlmEvent, LlmProvider, LlmRequest,
    ToolCall,
};
use crate::protocol::events::RuntimeEventPayload;
use crate::runtime::authority::{
    AuthorityError, ReviewResolution, RuntimeAuthority,
};
use crate::runtime::state::{PermissionState, RunId, RunStatus};

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

/// Per-run loop configuration. Cheap to clone; the underlying
/// provider/dispatcher are `Arc`-internal.
#[derive(Clone)]
pub struct LoopConfig {
    pub model: String,
    pub system_prompt: Option<String>,
    pub user_input: String,
    pub max_turns: usize,
    pub temperature: Option<f32>,
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<dyn ToolDispatcher>,
}

impl std::fmt::Debug for LoopConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopConfig")
            .field("model", &self.model)
            .field("system_prompt", &self.system_prompt)
            .field("user_input_len", &self.user_input.len())
            .field("max_turns", &self.max_turns)
            .field("temperature", &self.temperature)
            .field("llm", &"<provider>")
            .field("tools", &"<dispatcher>")
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
}

impl ReactLoop {
    pub fn new(
        authority: RuntimeAuthority,
        run_id: RunId,
        config: LoopConfig,
    ) -> Self {
        let mut history = Vec::with_capacity(2);
        if let Some(sys) = &config.system_prompt {
            history.push(ChatMessage::system(sys.clone()));
        }
        history.push(ChatMessage::user(config.user_input.clone()));
        Self {
            authority,
            run_id,
            config,
            history,
            turn: 0,
        }
    }

    /// Drive the loop to completion (success, failure, or cancellation).
    /// Errors flow through both the returned `Result` *and* a `Failed`
    /// run-status transition so subscribers see them.
    pub async fn run(mut self) -> Result<(), LoopError> {
        // Promote Pending -> Running. Subscribers see this as a
        // RunStatus event with status="running".
        self.authority.mark_run_running(self.run_id)?;

        let outcome = self.drive().await;
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

            let req = LlmRequest {
                model: self.config.model.clone(),
                messages: self.history.clone(),
                tools: self.config.tools.definitions(),
                temperature: self.config.temperature,
            };

            let mut stream = self
                .config
                .llm
                .stream(req)
                .await
                .map_err(|e| { info!(run = %self.run_id, error = %e, "llm stream failed"); LoopError::Provider(e) })?;

            let mut text = String::new();
            let mut calls: BTreeMap<usize, AccumCall> = BTreeMap::new();
            let mut finish: Option<FinishReason> = None;

            while let Some(event) = stream.next().await {
                match event {
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
                        if let Some(id) = id {
                            entry.id = id;
                        }
                        if let Some(name) = name {
                            entry.name = name;
                        }
                        if let Some(chunk) = arguments_chunk {
                            entry.arguments.push_str(&chunk);
                        }
                    }
                    LlmEvent::Done { finish_reason } => {
                        finish = Some(finish_reason);
                        break;
                    }
                    LlmEvent::Error { message } => {
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
            // Surface a structured trace entry for replay/UI.
            self.authority.emit_for_run(
                self.run_id,
                RuntimeEventPayload::Trace {
                    run_id: self.run_id.to_string(),
                    trace: serde_json::json!({
                        "kind": "assistant_turn",
                        "turn": turn_id,
                        "text": text,
                        "tool_calls": tool_calls.iter().map(|c| serde_json::json!({
                            "id": c.id, "name": c.name, "arguments": c.arguments,
                        })).collect::<Vec<_>>(),
                        "finish_reason": finish,
                    }),
                },
            );

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
                    let mut terminal_seen = false;
                    for call in &tool_calls {
                        match self.run_one_tool(call).await? {
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

    async fn run_one_tool(&mut self, call: &ToolCall) -> Result<ToolStepResult, LoopError> {
        // Surface the tool start as a trace event for the UI.
        self.authority.emit_for_run(
            self.run_id,
            RuntimeEventPayload::Trace {
                run_id: self.run_id.to_string(),
                trace: serde_json::json!({
                    "kind": "tool_start",
                    "tool": call.name,
                    "tool_call_id": call.id,
                    "arguments": call.arguments,
                }),
            },
        );

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

        let (result_value, terminal) = match outcome {
            ToolOutcome::Output(v) => (v, false),
            ToolOutcome::Error(e) => (serde_json::json!({"error": e}), false),
            ToolOutcome::Terminal(v) => (v, true),
            ToolOutcome::NeedsApproval { .. } => {
                // dispatch_approved should never re-return NeedsApproval.
                (serde_json::json!({"error": "tool repeatedly needs approval"}), false)
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
        self.authority.emit_for_run(
            self.run_id,
            RuntimeEventPayload::Trace {
                run_id: self.run_id.to_string(),
                trace: serde_json::json!({
                    "kind": "tool_done",
                    "tool": call.name,
                    "tool_call_id": call.id,
                    "result": result_value,
                    "terminal": terminal,
                }),
            },
        );
        if terminal {
            Ok(ToolStepResult::Terminal)
        } else {
            Ok(ToolStepResult::Continue)
        }
    }
}

#[derive(Default, Debug)]
struct AccumCall {
    id: String,
    name: String,
    arguments: String,
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
    use crate::runtime::state::{PermissionState, ReviewId, Space, SpaceId};
    use crate::runtime::SubscribeOutcome;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_authority() -> (RuntimeAuthority, SpaceId) {
        let auth = RuntimeAuthority::in_memory();
        let space = Space { id: SpaceId::new(), name: "test".into() };
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
            llm,
            tools,
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
        ReactLoop::new(auth.clone(), run_id, cfg).run().await.unwrap();

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
        let SubscribeOutcome { id: _, mut receiver } =
            auth.subscribe(format!("run:{run_id}"));
        let cfg = make_config(llm.clone(), Arc::new(EchoDispatcher));
        ReactLoop::new(auth.clone(), run_id, cfg).run().await.unwrap();
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
        ReactLoop::new(auth.clone(), run_id, cfg).run().await.unwrap();

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
        ReactLoop::new(auth.clone(), run_id, cfg).run().await.unwrap();

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
        llm.push(MockScript::new().delta("Approved and done.").done(FinishReason::Stop));

        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        // Subscribe to watch for PermissionRequest events.
        let SubscribeOutcome { id: _, mut receiver } =
            auth.subscribe(format!("run:{run_id}"));

        let auth2 = auth.clone();
        let cfg = make_config(llm.clone(), Arc::new(ApprovalDispatcher));
        let loop_task = tokio::spawn(async move {
            ReactLoop::new(auth2, run_id, cfg).run().await
        });

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
        llm.push(MockScript::new().delta("Understood.").done(FinishReason::Stop));

        let run_id = auth.start_run(sid, None, serde_json::json!({})).unwrap();
        let SubscribeOutcome { id: _, mut receiver } =
            auth.subscribe(format!("run:{run_id}"));

        let auth2 = auth.clone();
        let cfg = make_config(llm.clone(), Arc::new(ApprovalDispatcher));
        let loop_task = tokio::spawn(async move {
            ReactLoop::new(auth2, run_id, cfg).run().await
        });

        let review_id: ReviewId = loop {
            let ev = receiver.recv().await.expect("subscription channel closed");
            match ev.payload {
                RuntimeEventPayload::PermissionRequest { review_id, .. } => {
                    break ReviewId(Uuid::parse_str(&review_id).expect("valid uuid"));
                }
                _ => {}
            }
        };

        auth.resolve_review(run_id, review_id, PermissionState::Rejected, Some("not allowed".into()))
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
}
