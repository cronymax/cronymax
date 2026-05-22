//! Events surface — runtime-emitted, append-only facts streamed to
//! subscribers.
//!
//! These are the *only* messages that flow runtime → host on the events
//! channel. Hosts never invent semantic events; they project the ones
//! the runtime emits onto UI state.
//!
//! Concrete event payloads stay deliberately open at the protocol layer
//! so tasks 4.x / 5.x can fill them in without renegotiating the wire
//! format.

use serde::{Deserialize, Serialize};

/// Top-level runtime event. Carries a monotonically-increasing
/// `sequence` number per subscription so hosts can detect gaps after a
/// reconnect, and an emit timestamp for ordering across subscriptions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeEvent {
    /// Per-subscription monotonic sequence. Starts at 0 for the first
    /// event delivered after `Subscribed`.
    pub sequence: u64,

    /// Wall-clock time the runtime emitted the event, in milliseconds
    /// since UNIX epoch. Advisory only — `sequence` is the ordering
    /// authority within a subscription.
    pub emitted_at_ms: i64,

    /// Concrete event payload.
    pub payload: RuntimeEventPayload,
}

/// Variant body of a [`RuntimeEvent`].
///
/// New variants MUST be added; existing tags MUST NOT be repurposed.
/// Unknown variants are forward-compatibility errors on the host side.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeEventPayload {
    /// Run lifecycle transition (created, running, paused, succeeded,
    /// failed, cancelled, awaiting_review, ...).
    RunStatus {
        run_id: String,
        status: String,
        /// Agent identifier for this run. For flow node sub-runs this is
        /// the node's agent name (e.g. `"pm-design"`); for top-level chat
        /// runs it is the UUID-string of the authority agent, or `None`.
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        /// Flow run this sub-run belongs to, or `None` for top-level runs.
        #[serde(skip_serializing_if = "Option::is_none")]
        flow_run_id: Option<String>,
        detail: Option<serde_json::Value>,
    },

    /// Trace event — orchestrator step, tool call, tool result, model
    /// turn, etc. The payload schema is owned by `cronygraph` /
    /// `cronymax::trace` and lands with task 4.3.
    Trace {
        run_id: String,
        trace: serde_json::Value,
    },

    /// Streaming token delta from a model turn.
    Token {
        run_id: String,
        turn_id: String,
        delta: String,
    },

    /// Streaming thinking/reasoning token delta from a model turn.
    /// Emitted before `Token` events for the same turn when the model
    /// supports extended thinking. Thinking content is ephemeral — it is
    /// never stored in conversation history.
    ThinkingToken {
        run_id: String,
        turn_id: String,
        delta: String,
    },

    /// Permission/review prompt the runtime is waiting on.
    PermissionRequest {
        run_id: String,
        review_id: String,
        request: serde_json::Value,
    },

    /// Free-form runtime log line surfaced to UI for diagnostics.
    Log {
        level: LogLevel,
        target: String,
        message: String,
    },

    /// Generic raw payload — used for terminal output and other
    /// non-structured events that carry opaque JSON data.
    Raw { data: serde_json::Value },

    /// A file was modified by the agent via `str_replace` or `write_file`.
    FileEdited {
        run_id: String,
        session_id: Option<String>,
        path: String,
        /// Unified diff of the change (empty for write_file).
        diff: String,
    },

    /// A git commit was created by the agent.
    GitCommitCreated {
        run_id: String,
        session_id: Option<String>,
        hash: String,
        message: String,
        files_changed: Vec<String>,
    },

    /// A git push was completed by the agent.
    GitPushed {
        run_id: String,
        session_id: Option<String>,
        remote: String,
        branch: String,
        commits_pushed: usize,
    },

    /// A Supervisor-dispatched child task has started running.
    TaskStarted { run_id: String, task_id: String },

    /// A Supervisor-dispatched child task has finished (succeeded or failed).
    TaskCompleted {
        run_id: String,
        task_id: String,
        success: bool,
    },

    // ── supervisor-session-ux ─────────────────────────────────────────────
    /// Emitted after a session is auto-named or manually renamed.
    /// The sidebar listens for this to update the session label.
    SessionRenamed {
        session_id: String,
        /// The new display name.
        name: String,
        /// `true` if the rename was triggered by the user (manual rename from
        /// the sidebar), `false` if it was an auto-name after the first
        /// invocation completion.
        manually_named: bool,
    },

    /// Emitted after each critic pass for an agent run (task 9.3).
    /// The AgentThreadView subscribes to show a CriticPassBanner inline.
    CriticResult {
        run_id: String,
        /// Name of the agent whose output was critiqued (e.g. "code").
        agent_name: String,
        /// `true` if the critic accepted the output; `false` if revision was requested.
        passed: bool,
        /// Short summary / issues list from the critic (empty if passed).
        summary: String,
        /// Which revision number this is (1-based).
        revision: u32,
        /// Maximum revisions allowed for this run.
        max_revisions: u32,
    },
}

/// Severity for `RuntimeEventPayload::Log`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
