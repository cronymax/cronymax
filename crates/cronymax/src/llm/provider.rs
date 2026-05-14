//! Provider trait + streaming event types.
//!
//! A provider takes an [`LlmRequest`] and returns a stream of
//! [`LlmEvent`]s. The agent loop consumes that stream, accumulates
//! tool-call deltas by index (matching the OpenAI streaming convention
//! the renderer used in `web/src/agent_runtime/llm.js`), and decides
//! whether to dispatch tools or terminate the turn.

use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;

use super::messages::{FinishReason, LlmRequest};

/// One event in a streaming completion.
#[derive(Clone, Debug)]
pub enum LlmEvent {
    /// A chunk of assistant text. Concatenate all `Delta.content` to
    /// reconstruct the assistant's message.
    Delta { content: String },
    /// A chunk of thinking/reasoning content emitted by a model that
    /// supports extended thinking (Anthropic claude-*, OpenAI o-series
    /// via `reasoning_content`). Thinking chunks precede `Delta` chunks
    /// within a turn. They are accumulated and emitted as
    /// `RuntimeEventPayload::ThinkingToken` but never stored in history.
    ThinkingDelta { content: String },
    /// A streaming chunk of a tool-call. `index` keys per-call
    /// accumulators; provider may emit `id`/`name` only on the first
    /// chunk and stream `arguments_chunk` thereafter.
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_chunk: Option<String>,
    },
    /// Token usage for the current turn. Emitted by providers that
    /// surface usage in the stream (Anthropic via `message_start` +
    /// `message_delta`; OpenAI via final chunk `usage` field). Multiple
    /// `Usage` events may arrive per stream; the agent loop accumulates them.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// Turn completed. `Stop` and `Length` mean the loop is done;
    /// `ToolCalls` means the loop should now dispatch the accumulated
    /// tool calls and continue.
    Done { finish_reason: FinishReason },
    /// Non-fatal stream-side error. The loop will surface this as a
    /// turn failure.
    Error { message: String },
}

/// Boxed stream alias used by every provider impl. `'static` so it
/// can outlive the request handle and be moved across tasks.
pub type LlmStream = Pin<Box<dyn Stream<Item = LlmEvent> + Send + 'static>>;

/// Provider-facing entry point. Implementations own their own HTTP
/// client / scripted state.
#[async_trait]
pub trait LlmProvider: Send + Sync + std::fmt::Debug {
    /// Issue a streaming completion. Errors here are setup failures
    /// (bad config, network unreachable). Per-stream errors arrive as
    /// [`LlmEvent::Error`] inside the returned stream.
    async fn stream(&self, request: LlmRequest) -> anyhow::Result<LlmStream>;
}
