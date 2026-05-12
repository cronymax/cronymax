//! In-process scripted [`LlmProvider`] used by tests and as a no-op
//! fallback when no real provider is configured.
//!
//! A [`MockScript`] is just an ordered list of [`ScriptStep`]s. Each
//! call to [`MockLlmProvider::stream`] consumes one script (popped
//! from a queue) and emits its steps in order as [`LlmEvent`]s. This
//! matches what real providers do (a sequence of deltas terminated by
//! a `Done` event) without going anywhere near a network.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::messages::{FinishReason, LlmRequest};
use super::provider::{LlmEvent, LlmProvider, LlmStream};
use super::stream::UnboundedReceiverStream;

/// One emitted event in a scripted turn. Mirrors [`LlmEvent`] but
/// owned (the trait variant is `Clone` already so we just reuse it).
pub type ScriptStep = LlmEvent;

/// An ordered sequence of [`ScriptStep`]s representing one model turn.
#[derive(Clone, Debug, Default)]
pub struct MockScript {
    pub steps: Vec<ScriptStep>,
}

impl MockScript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn delta(mut self, content: impl Into<String>) -> Self {
        self.steps.push(LlmEvent::Delta {
            content: content.into(),
        });
        self
    }

    pub fn tool_call(
        mut self,
        index: usize,
        id: impl Into<String>,
        name: impl Into<String>,
        arguments_json: impl Into<String>,
    ) -> Self {
        // Single-shot: emit id+name+full-arguments in one delta. That's
        // a legal subset of the streaming protocol.
        self.steps.push(LlmEvent::ToolCallDelta {
            index,
            id: Some(id.into()),
            name: Some(name.into()),
            arguments_chunk: Some(arguments_json.into()),
        });
        self
    }

    pub fn done(mut self, reason: FinishReason) -> Self {
        self.steps.push(LlmEvent::Done {
            finish_reason: reason,
        });
        self
    }
}

/// Pop-a-script-per-call provider. Cheap to clone (`Arc`-internal).
#[derive(Clone, Debug, Default)]
pub struct MockLlmProvider {
    inner: Arc<Mutex<MockInner>>,
}

#[derive(Debug, Default)]
struct MockInner {
    scripts: VecDeque<MockScript>,
    /// Captured requests for assertions.
    seen: Vec<LlmRequest>,
}

impl MockLlmProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a script to be returned by the next [`stream`] call.
    pub fn push(&self, script: MockScript) {
        self.inner.lock().scripts.push_back(script);
    }

    /// Snapshot of every request seen so far. Useful for asserting
    /// that the loop sent the right tool results back.
    pub fn requests(&self) -> Vec<LlmRequest> {
        self.inner.lock().seen.clone()
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn stream(&self, request: LlmRequest) -> anyhow::Result<LlmStream> {
        let mut inner = self.inner.lock();
        inner.seen.push(request);
        let script = inner.scripts.pop_front().unwrap_or_else(|| {
            // No script queued -> default to a Stop. Keeps tests that
            // forget to script a final turn from hanging.
            MockScript::new().done(FinishReason::Stop)
        });
        drop(inner);
        let (tx, rx) = mpsc::unbounded_channel();
        for step in script.steps {
            let _ = tx.send(step);
        }
        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }
}
