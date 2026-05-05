//! Control surface — host-initiated semantic mutations and queries
//! against the runtime authority.
//!
//! This is the only legitimate path for the host to *change* runtime
//! state. Direct persistence writes or in-host orchestration are
//! explicitly disallowed by the migration design.
//!
//! Concrete request payloads are intentionally minimal at this stage:
//! tasks 4.x flesh out run / agent / review semantics. The variant
//! shapes here are designed so additional fields can be appended
//! without changing the wire tag.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::envelope::SubscriptionId;

/// Host-initiated control message.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlRequest {
    /// Liveness ping. Runtime replies with `ControlResponse::Pong`.
    Ping,

    /// Subscribe to runtime events.
    ///
    /// `topic` is opaque to the protocol — concrete topics ("run:<id>",
    /// "space:<id>/inbox", etc.) are defined alongside the events that
    /// populate them in tasks 4.x / 5.x.
    Subscribe { topic: String },

    /// Tear down a previously-opened subscription.
    Unsubscribe { subscription: SubscriptionId },

    /// Start a new run inside the given Space.
    ///
    /// `payload` is JSON-shaped at this layer; concrete fields land in
    /// task 4.2 once `RunSpec` is defined in `cronymax::runs`.
    StartRun {
        space_id: String,
        payload: serde_json::Value,
    },

    /// Cancel an in-flight run.
    CancelRun { run_id: String },

    /// Pause an in-flight run (cooperative).
    PauseRun { run_id: String },

    /// Resume a paused or awaiting-approval run.
    ResumeRun { run_id: String },

    /// Post user input into a running conversation / waiting prompt.
    PostInput {
        run_id: String,
        payload: serde_json::Value,
    },

    /// Resolve a pending review/permission decision.
    ResolveReview {
        run_id: String,
        review_id: String,
        decision: ReviewDecision,
        notes: Option<String>,
    },
}

/// Reply to a [`ControlRequest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlResponse {
    Pong,

    /// Returned in reply to `Subscribe`. The host stores the id and
    /// pairs incoming events to the originating UI surface.
    Subscribed { subscription: SubscriptionId },

    /// Returned in reply to `Unsubscribe`.
    Unsubscribed,

    /// Returned in reply to `StartRun`. `subscription` is an auto-created
    /// subscription for the run's event stream so the host can register its
    /// event listener before any events can arrive.
    RunStarted { run_id: String, subscription: SubscriptionId },

    /// Acknowledgement for mutating commands that don't return data.
    Ack,

    /// Generic failure envelope. The runtime always prefers a typed
    /// `Err` over closing the connection so the host can report cleanly.
    Err { error: ControlError },
}

/// Decision values for `ControlRequest::ResolveReview`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Reject,
    Defer,
}

/// Typed error returned in `ControlResponse::Err`.
#[derive(Clone, Debug, Serialize, Deserialize, Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ControlError {
    #[error("unknown run: {run_id}")]
    UnknownRun { run_id: String },

    #[error("unknown space: {space_id}")]
    UnknownSpace { space_id: String },

    #[error("unknown subscription")]
    UnknownSubscription,

    #[error("invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("operation not allowed in current state: {message}")]
    InvalidState { message: String },

    #[error("internal runtime error: {message}")]
    Internal { message: String },
}
