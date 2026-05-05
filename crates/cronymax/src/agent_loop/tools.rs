//! Tool dispatch trait + supporting types.
//!
//! The loop is tool-registry-agnostic: it just hands a [`ToolCall`]
//! to a [`ToolDispatcher`] and switches on the [`ToolOutcome`].
//! Concrete implementations (host capability adapters in task 6.x,
//! local in-process tools, scripted mocks) plug in via `Arc<dyn ...>`.

use async_trait::async_trait;

use crate::llm::{ToolCall, ToolDef};

/// Outcome of dispatching a single tool call.
#[derive(Clone, Debug)]
pub enum ToolOutcome {
    /// Tool ran and produced a structured result. Serialized into the
    /// `role: tool` message for the next LLM turn.
    Output(serde_json::Value),
    /// Tool ran but failed. Reported back to the model as a tool
    /// message so it can recover (matches the renderer's behavior).
    Error(String),
    /// Tool requires a user approval before it can run. The loop will
    /// open a runtime review and pause the run; on approval, the
    /// dispatcher is called again via [`ToolDispatcher::dispatch_approved`].
    NeedsApproval { request: serde_json::Value },
    /// Terminal tool (e.g. `submit_document`): record the result and
    /// stop the loop with `Succeeded`.
    Terminal(serde_json::Value),
}

/// What the loop sees through. Implementations are expected to be
/// cheap to clone (typically `Arc`-internal).
#[async_trait]
pub trait ToolDispatcher: Send + Sync + std::fmt::Debug {
    /// Tool definitions advertised to the LLM on every turn.
    fn definitions(&self) -> Vec<ToolDef>;

    /// Dispatch a tool call. Called once per call per turn.
    async fn dispatch(&self, call: &ToolCall) -> ToolOutcome;

    /// Re-dispatch after a `NeedsApproval` review was approved. The
    /// default just calls [`dispatch`] again — override if the second
    /// path needs different state (e.g. a "now actually run it" flag).
    async fn dispatch_approved(&self, call: &ToolCall) -> ToolOutcome {
        self.dispatch(call).await
    }
}

/// Empty dispatcher used when no tools are configured. Treats every
/// call as a no-such-tool error so the model doesn't loop forever.
#[derive(Clone, Debug, Default)]
pub struct EmptyDispatcher;

#[async_trait]
impl ToolDispatcher for EmptyDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        Vec::new()
    }

    async fn dispatch(&self, call: &ToolCall) -> ToolOutcome {
        ToolOutcome::Error(format!("no tool registered: {}", call.name))
    }
}
