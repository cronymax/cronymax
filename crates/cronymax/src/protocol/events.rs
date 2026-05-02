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
