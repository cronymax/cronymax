//! Common envelope, channel, and correlation primitives shared by all
//! three protocol surfaces (control, events, capabilities).
//!
//! Wire format is JSON for now (chosen for legibility + tooling). When
//! the real GIPS transport is wired in task 2.2, payload framing moves
//! to its native message boundary; the *shape* defined here stays.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::capabilities::{CapabilityRequest, CapabilityResponse};
use super::control::{ControlRequest, ControlResponse};
use super::events::RuntimeEvent;
use super::version::ProtocolVersion;

/// Stable identifier for a single request/response pair across the
/// boundary. The originating side mints the id; the responder echoes it
/// verbatim. Generated as UUID v4.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CorrelationId(Uuid);

impl CorrelationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for CorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifier for an active event subscription. The host opens a
/// subscription via [`ControlRequest::Subscribe`] and keeps the id to
/// pair incoming events back to the originating UI surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubscriptionId(Uuid);

impl SubscriptionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SubscriptionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Logical channel a message belongs to. Carried in the envelope so a
/// single underlying GIPS connection can multiplex all three surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Control,
    Events,
    Capabilities,
}

/// All messages the host (or any client) can send to the runtime.
///
/// `tag` discriminates the surface; payload is one of:
///   * a `Hello` handshake (mandatory first message),
///   * a control request,
///   * a capability response (host fulfilling a runtime-initiated call).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
pub enum ClientToRuntime {
    /// Initial handshake declaring the host-supported protocol version.
    /// Must be the first message on a fresh connection.
    Hello {
        protocol: ProtocolVersion,
        client_name: String,
        client_version: String,
    },
    /// Host-initiated control request expecting a [`ControlResponse`].
    Control {
        id: CorrelationId,
        request: ControlRequest,
    },
    /// Host fulfilling a previously-issued capability request.
    CapabilityReply {
        id: CorrelationId,
        response: CapabilityResponse,
    },
}

/// All messages the runtime can send to the host.
///
/// `tag` discriminates the surface; payload is one of:
///   * a `Welcome` handshake reply,
///   * a control response,
///   * a streamed runtime event,
///   * a capability invocation request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
pub enum RuntimeToClient {
    /// Handshake reply. Sent in response to [`ClientToRuntime::Hello`].
    /// On version mismatch the runtime sends [`Self::Goodbye`] instead.
    Welcome {
        protocol: ProtocolVersion,
        runtime_version: String,
    },
    /// Connection is being closed. Always terminal.
    Goodbye {
        reason: GoodbyeReason,
        message: String,
    },
    /// Reply to a control request.
    Control {
        id: CorrelationId,
        response: ControlResponse,
    },
    /// Runtime-emitted event for an active subscription.
    Event {
        subscription: SubscriptionId,
        event: RuntimeEvent,
    },
    /// Request the host to perform a privileged capability call.
    /// The host MUST eventually reply with
    /// [`ClientToRuntime::CapabilityReply`] using the same correlation id.
    CapabilityCall {
        id: CorrelationId,
        request: CapabilityRequest,
    },
}

/// Why a connection is being closed by the runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoodbyeReason {
    ProtocolMismatch,
    HandshakeRequired,
    ProtocolViolation,
    Shutdown,
}
