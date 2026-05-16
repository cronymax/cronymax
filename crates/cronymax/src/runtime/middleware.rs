//! Run-middleware system for intercepting ReactLoop lifecycle events.
//!
//! Provides:
//! - [`TokenUsage`] — per-turn token count accumulator.
//! - [`TurnContext`] — context passed to every hook.
//! - [`ToolGate`] — `before_tool_call` return type (allow / block).
//! - [`RunMiddleware`] — async trait with six lifecycle hooks.
//! - [`MiddlewareChain`] — ordered chain; `before_tool_call` short-circuits
//!   on the first `Block`.
//! - [`TimingMiddleware`] — records wall-clock duration for LLM and tool
//!   calls; exposes durations via shared `Arc` stores.
//! - [`TokenAccumulatorMiddleware`] — accumulates `LlmEvent::Usage` values
//!   into `TurnContext.total_usage` across turns.
//! - [`TraceEmitterMiddleware`] — emits all structured trace events via
//!   `RuntimeAuthority::emit_for_run`, replacing the inline calls that were
//!   previously scattered throughout `ReactLoop`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::agent_loop::tools::ToolOutcome;
use crate::llm::{FinishReason, ToolCall};
use crate::protocol::events::RuntimeEventPayload;
use crate::runtime::authority::RuntimeAuthority;
use crate::runtime::state::RunId;

// ── Token usage ───────────────────────────────────────────────────────────────

/// Accumulated input/output token counts for one or more LLM turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl std::ops::Add for TokenUsage {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
        }
    }
}

impl std::ops::AddAssign for TokenUsage {
    fn add_assign(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

// ── Turn context ──────────────────────────────────────────────────────────────

/// Per-turn context passed to every middleware hook.
///
/// `total_usage` reflects token counts accumulated across all turns that
/// completed **before** the current one. `TokenAccumulatorMiddleware` fills
/// this in during `before_llm_call` so all subsequent hooks see the correct
/// cumulative value.
#[derive(Clone, Debug)]
pub struct TurnContext {
    pub run_id: RunId,
    pub turn: u64,
    pub model: String,
    /// Token usage accumulated across all turns so far (not including the
    /// current in-flight turn). Set by `TokenAccumulatorMiddleware` in
    /// `before_llm_call`.
    pub total_usage: TokenUsage,
}

// ── Tool gate ─────────────────────────────────────────────────────────────────

/// Return value of [`RunMiddleware::before_tool_call`].
///
/// `Allow` lets the dispatch proceed; `Block(reason)` short-circuits the
/// chain and causes `ReactLoop` to record `{"error": reason}` as the tool
/// result without dispatching the tool.
#[derive(Debug)]
pub enum ToolGate {
    Allow,
    Block(String),
}

// ── RunMiddleware trait ───────────────────────────────────────────────────────

/// Async lifecycle trait for intercepting `ReactLoop` events.
///
/// All methods have default no-op implementations so middleware only needs
/// to override the hooks it cares about.
#[async_trait]
pub trait RunMiddleware: Send + Sync {
    /// Called once per loop iteration, just before the LLM request is issued.
    /// Receives a *mutable* context so hooks can update `total_usage` etc.
    async fn before_llm_call(&self, _ctx: &mut TurnContext) {}

    /// Called after the LLM stream is fully consumed, before tool dispatch.
    ///
    /// `text` is the full assistant message text for this turn.
    /// `finish_reason` is the serialized finish reason (e.g. `"stop"`,
    /// `"tool_calls"`).
    /// `usage` is the token usage accumulated from the stream for this turn.
    async fn after_llm_call(
        &self,
        _ctx: &TurnContext,
        _text: &str,
        _finish_reason: &FinishReason,
        _usage: &TokenUsage,
    ) {
    }

    /// Called before a tool is dispatched. Return `ToolGate::Block(reason)`
    /// to prevent dispatch; `ToolGate::Allow` to proceed.
    async fn before_tool_call(&self, _ctx: &TurnContext, _call: &ToolCall) -> ToolGate {
        ToolGate::Allow
    }

    /// Called after a tool returns (or is blocked). `outcome` is the resolved
    /// `ToolOutcome` — never `NeedsApproval` (approval is handled in
    /// `ReactLoop` before this hook is invoked). For blocked calls the outcome
    /// is `ToolOutcome::Error(reason)`.
    async fn after_tool_call(&self, _ctx: &TurnContext, _call: &ToolCall, _outcome: &ToolOutcome) {}

    /// Called before the in-loop reflection pass runs.
    async fn before_reflection(&self, _ctx: &TurnContext) {}

    /// Called after the reflection pass produces its summary text.
    async fn after_reflection(&self, _ctx: &TurnContext, _text: &str) {}
}

// ── MiddlewareChain ───────────────────────────────────────────────────────────

/// Ordered list of middleware. Each hook is called on every entry in
/// registration order. `before_tool_call` short-circuits on the first
/// `Block`; all other hooks run all entries unconditionally.
pub struct MiddlewareChain(pub Vec<Arc<dyn RunMiddleware>>);

impl MiddlewareChain {
    /// Construct an empty chain (no-op for all hooks).
    pub fn empty() -> Self {
        Self(vec![])
    }

    pub async fn before_llm_call(&self, ctx: &mut TurnContext) {
        for mw in &self.0 {
            mw.before_llm_call(ctx).await;
        }
    }

    pub async fn after_llm_call(
        &self,
        ctx: &TurnContext,
        text: &str,
        finish_reason: &FinishReason,
        usage: &TokenUsage,
    ) {
        for mw in &self.0 {
            mw.after_llm_call(ctx, text, finish_reason, usage).await;
        }
    }

    /// Returns `ToolGate::Allow` when all middleware allow; returns the first
    /// `Block` and stops evaluating remaining middleware.
    pub async fn before_tool_call(&self, ctx: &TurnContext, call: &ToolCall) -> ToolGate {
        for mw in &self.0 {
            if let ToolGate::Block(reason) = mw.before_tool_call(ctx, call).await {
                return ToolGate::Block(reason);
            }
        }
        ToolGate::Allow
    }

    pub async fn after_tool_call(&self, ctx: &TurnContext, call: &ToolCall, outcome: &ToolOutcome) {
        for mw in &self.0 {
            mw.after_tool_call(ctx, call, outcome).await;
        }
    }

    pub async fn before_reflection(&self, ctx: &TurnContext) {
        for mw in &self.0 {
            mw.before_reflection(ctx).await;
        }
    }

    pub async fn after_reflection(&self, ctx: &TurnContext, text: &str) {
        for mw in &self.0 {
            mw.after_reflection(ctx, text).await;
        }
    }
}

// ── Shared duration stores ────────────────────────────────────────────────────

/// LLM call durations in milliseconds, keyed by `(run_id, turn)`.
/// Written by [`TimingMiddleware`]; read by [`TraceEmitterMiddleware`].
pub type LlmDurationStore = Arc<Mutex<HashMap<(RunId, u64), u64>>>;

/// Tool call durations in milliseconds, keyed by `tool_call_id`.
/// Written by [`TimingMiddleware`]; read by [`TraceEmitterMiddleware`].
pub type ToolDurationStore = Arc<Mutex<HashMap<String, u64>>>;

// ── TimingMiddleware ──────────────────────────────────────────────────────────

/// Records wall-clock duration for each LLM call and tool execution.
///
/// Start times are stored internally; computed durations are written to
/// the shared [`LlmDurationStore`] / [`ToolDurationStore`] so
/// [`TraceEmitterMiddleware`] (which runs after in the default chain) can
/// include them in emitted trace events.
pub struct TimingMiddleware {
    llm_start: Arc<Mutex<HashMap<(RunId, u64), Instant>>>,
    tool_start: Arc<Mutex<HashMap<String, Instant>>>,
    /// Computed LLM-call durations; shared with `TraceEmitterMiddleware`.
    pub llm_durations: LlmDurationStore,
    /// Computed tool-call durations; shared with `TraceEmitterMiddleware`.
    pub tool_durations: ToolDurationStore,
}

impl TimingMiddleware {
    pub fn new(llm_durations: LlmDurationStore, tool_durations: ToolDurationStore) -> Self {
        Self {
            llm_start: Default::default(),
            tool_start: Default::default(),
            llm_durations,
            tool_durations,
        }
    }
}

#[async_trait]
impl RunMiddleware for TimingMiddleware {
    async fn before_llm_call(&self, ctx: &mut TurnContext) {
        self.llm_start
            .lock()
            .insert((ctx.run_id, ctx.turn), Instant::now());
    }

    async fn after_llm_call(
        &self,
        ctx: &TurnContext,
        _text: &str,
        _finish_reason: &FinishReason,
        _usage: &TokenUsage,
    ) {
        if let Some(start) = self.llm_start.lock().remove(&(ctx.run_id, ctx.turn)) {
            let ms = start.elapsed().as_millis() as u64;
            self.llm_durations.lock().insert((ctx.run_id, ctx.turn), ms);
        }
    }

    async fn before_tool_call(&self, _ctx: &TurnContext, call: &ToolCall) -> ToolGate {
        self.tool_start
            .lock()
            .insert(call.id.clone(), Instant::now());
        ToolGate::Allow
    }

    async fn after_tool_call(&self, _ctx: &TurnContext, call: &ToolCall, _outcome: &ToolOutcome) {
        if let Some(start) = self.tool_start.lock().remove(&call.id) {
            let ms = start.elapsed().as_millis() as u64;
            self.tool_durations.lock().insert(call.id.clone(), ms);
        }
    }
}

// ── TokenAccumulatorMiddleware ────────────────────────────────────────────────

/// Accumulates per-turn `LlmEvent::Usage` values into per-run totals.
///
/// In `before_llm_call` it sets `ctx.total_usage` to the accumulated total
/// from all previously completed turns. In `after_llm_call` it adds the
/// current turn's usage to the accumulator so the next turn sees it.
pub struct TokenAccumulatorMiddleware {
    per_run: Arc<Mutex<HashMap<RunId, TokenUsage>>>,
}

impl TokenAccumulatorMiddleware {
    pub fn new() -> Self {
        Self {
            per_run: Default::default(),
        }
    }
}

impl Default for TokenAccumulatorMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RunMiddleware for TokenAccumulatorMiddleware {
    async fn before_llm_call(&self, ctx: &mut TurnContext) {
        // Fill in total_usage from previously completed turns.
        let store = self.per_run.lock();
        ctx.total_usage = store.get(&ctx.run_id).copied().unwrap_or_default();
    }

    async fn after_llm_call(
        &self,
        ctx: &TurnContext,
        _text: &str,
        _finish_reason: &FinishReason,
        usage: &TokenUsage,
    ) {
        // Accumulate this turn's usage for future turns.
        let mut store = self.per_run.lock();
        *store.entry(ctx.run_id).or_default() += *usage;
    }
}

// ── TraceEmitterMiddleware ────────────────────────────────────────────────────

/// Emits all structured trace events via `RuntimeAuthority::emit_for_run`.
///
/// Replaces the inline `emit_for_run` calls that were previously scattered
/// throughout `ReactLoop`. Enriches `assistant_turn` and `tool_done` traces
/// with `duration_ms` (from `TimingMiddleware`'s shared stores) and
/// `usage` (from the per-turn `TokenUsage` argument).
pub struct TraceEmitterMiddleware {
    authority: Arc<RuntimeAuthority>,
    llm_durations: LlmDurationStore,
    tool_durations: ToolDurationStore,
}

impl TraceEmitterMiddleware {
    pub fn new(
        authority: Arc<RuntimeAuthority>,
        llm_durations: LlmDurationStore,
        tool_durations: ToolDurationStore,
    ) -> Self {
        Self {
            authority,
            llm_durations,
            tool_durations,
        }
    }
}

#[async_trait]
impl RunMiddleware for TraceEmitterMiddleware {
    async fn after_llm_call(
        &self,
        ctx: &TurnContext,
        text: &str,
        finish_reason: &FinishReason,
        usage: &TokenUsage,
    ) {
        let duration_ms = self
            .llm_durations
            .lock()
            .get(&(ctx.run_id, ctx.turn))
            .copied();

        let usage_val = if usage.input_tokens > 0 || usage.output_tokens > 0 {
            Some(serde_json::json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
            }))
        } else {
            None
        };

        let mut trace = serde_json::json!({
            "kind": "assistant_turn",
            "turn": ctx.turn,
            "text": text,
            "finish_reason": finish_reason,
        });

        if let Some(u) = usage_val {
            trace["usage"] = u;
        }
        if let Some(ms) = duration_ms {
            trace["duration_ms"] = serde_json::json!(ms);
        }

        self.authority.emit_for_run(
            ctx.run_id,
            RuntimeEventPayload::Trace {
                run_id: ctx.run_id.to_string(),
                trace,
            },
        );
    }

    async fn before_tool_call(&self, ctx: &TurnContext, call: &ToolCall) -> ToolGate {
        self.authority.emit_for_run(
            ctx.run_id,
            RuntimeEventPayload::Trace {
                run_id: ctx.run_id.to_string(),
                trace: serde_json::json!({
                    "kind": "tool_start",
                    "tool": call.name,
                    "tool_call_id": call.id,
                    "arguments": call.arguments,
                }),
            },
        );
        ToolGate::Allow
    }

    async fn after_tool_call(&self, ctx: &TurnContext, call: &ToolCall, outcome: &ToolOutcome) {
        let (result_value, terminal) = match outcome {
            ToolOutcome::Output(v) => (v.clone(), false),
            ToolOutcome::Error(e) => (serde_json::json!({"error": e}), false),
            ToolOutcome::Terminal(v) => (v.clone(), true),
            ToolOutcome::NeedsApproval { .. } => {
                // Should not reach here; approval is resolved before after_tool_call.
                (
                    serde_json::json!({"error": "unexpected NeedsApproval in after_tool_call"}),
                    false,
                )
            }
        };

        let duration_ms = self.tool_durations.lock().get(&call.id).copied();

        let mut trace = serde_json::json!({
            "kind": "tool_done",
            "tool": call.name,
            "tool_call_id": call.id,
            "result": result_value,
            "terminal": terminal,
        });

        if let Some(ms) = duration_ms {
            trace["duration_ms"] = serde_json::json!(ms);
        }

        self.authority.emit_for_run(
            ctx.run_id,
            RuntimeEventPayload::Trace {
                run_id: ctx.run_id.to_string(),
                trace,
            },
        );
    }

    async fn after_reflection(&self, ctx: &TurnContext, text: &str) {
        self.authority.emit_for_run(
            ctx.run_id,
            RuntimeEventPayload::Trace {
                run_id: ctx.run_id.to_string(),
                trace: serde_json::json!({
                    "kind": "reflection",
                    "turn": ctx.turn,
                    "text": text,
                }),
            },
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolCall;
    use crate::runtime::state::RunId;

    fn make_ctx(run_id: RunId, turn: u64) -> TurnContext {
        TurnContext {
            run_id,
            turn,
            model: "mock".into(),
            total_usage: TokenUsage::default(),
        }
    }

    fn make_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: "{}".into(),
        }
    }

    // ── 10.1: MiddlewareChain::before_tool_call short-circuits on Block ───────

    struct AllowMiddleware;
    struct BlockMiddleware(String);
    struct PanicMiddleware; // must never be called after a block

    #[async_trait::async_trait]
    impl RunMiddleware for AllowMiddleware {}

    #[async_trait::async_trait]
    impl RunMiddleware for BlockMiddleware {
        async fn before_tool_call(&self, _ctx: &TurnContext, _call: &ToolCall) -> ToolGate {
            ToolGate::Block(self.0.clone())
        }
    }

    #[async_trait::async_trait]
    impl RunMiddleware for PanicMiddleware {
        async fn before_tool_call(&self, _ctx: &TurnContext, _call: &ToolCall) -> ToolGate {
            panic!("PanicMiddleware::before_tool_call should not be called after a Block");
        }
    }

    #[tokio::test]
    async fn chain_before_tool_call_short_circuits_on_block() {
        let chain = MiddlewareChain(vec![
            Arc::new(AllowMiddleware),
            Arc::new(BlockMiddleware("rate limited".into())),
            Arc::new(PanicMiddleware),
        ]);
        let run_id = RunId::new();
        let ctx = make_ctx(run_id, 1);
        let call = make_call("id-1", "some_tool");
        let gate = chain.before_tool_call(&ctx, &call).await;
        assert!(
            matches!(gate, ToolGate::Block(ref r) if r == "rate limited"),
            "expected Block(rate limited), got {gate:?}"
        );
    }

    // ── 10.2: TokenAccumulatorMiddleware accumulates across after_llm_call ────

    #[tokio::test]
    async fn token_accumulator_accumulates_across_turns() {
        let accum = TokenAccumulatorMiddleware::new();
        let run_id = RunId::new();
        let finish = FinishReason::Stop;

        let usage1 = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
        };
        let usage2 = TokenUsage {
            input_tokens: 80,
            output_tokens: 40,
        };
        let usage3 = TokenUsage {
            input_tokens: 120,
            output_tokens: 60,
        };

        // Simulate three turns: call after_llm_call for each.
        accum
            .after_llm_call(&make_ctx(run_id, 1), "", &finish, &usage1)
            .await;
        accum
            .after_llm_call(&make_ctx(run_id, 2), "", &finish, &usage2)
            .await;
        accum
            .after_llm_call(&make_ctx(run_id, 3), "", &finish, &usage3)
            .await;

        // Turn 4: before_llm_call should set ctx.total_usage to the sum of all three.
        let mut ctx4 = make_ctx(run_id, 4);
        accum.before_llm_call(&mut ctx4).await;
        assert_eq!(
            ctx4.total_usage,
            TokenUsage {
                input_tokens: 300,
                output_tokens: 150,
            },
            "total_usage should be 300+150 after three turns"
        );
    }
}
