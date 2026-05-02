//! ReAct-style agent loop (task 5.1).
//!
//! Migrates the renderer's `web/src/agent_runtime/loop.js` driver into
//! the runtime crate so the host process is no longer the execution
//! authority. The loop:
//!
//! 1. Calls the configured [`crate::llm::LlmProvider`] with the
//!    current message history + tool definitions.
//! 2. Streams `Delta` events out as
//!    [`crate::protocol::events::RuntimeEventPayload::Token`]
//!    subscription events while accumulating the full assistant
//!    message and per-index tool-call deltas.
//! 3. On `finish_reason = Stop` (or a `Terminal` tool result) marks
//!    the run [`crate::runtime::state::RunStatus::Succeeded`].
//! 4. On `finish_reason = ToolCalls` it dispatches each call through
//!    a [`tools::ToolDispatcher`]:
//!      * `Output` → append a `role: tool` message and continue.
//!      * `NeedsApproval` → open a [`crate::runtime::PendingReview`],
//!        await `resolve_review`, dispatch again on approve.
//!      * `Error` → record the error as a tool result and continue
//!        (matches the renderer's behavior so the model can recover).
//!      * `Terminal` → record the result and finish with `Succeeded`.
//!
//! All loop output goes through [`crate::runtime::RuntimeAuthority`]
//! so subscribers see exactly the same event stream regardless of
//! which provider/dispatcher backed the run.

pub mod react;
pub mod tools;

pub use react::{LoopConfig, LoopError, ReactLoop};
pub use tools::{ToolDispatcher, ToolOutcome};
