//! Runtime <-> host protocol.
//!
//! The protocol is split into three logical surfaces that all flow over
//! a single GIPS transport (Decision 2 in the migration design):
//!
//! * [`control`] — host-initiated semantic mutations: start a run,
//!   cancel, post user input, approve a review, query state.
//! * [`events`] — runtime-initiated, append-only facts streamed to the
//!   host: run lifecycle changes, trace events, token deltas,
//!   permission requests.
//! * [`capabilities`] — runtime-initiated, host-fulfilled privileged
//!   operations (PTY, browser, FS, notifications, secrets) returned via
//!   correlated responses.
//!
//! The transport layer ([`transport`]) and dispatch glue ([`dispatch`])
//! are deliberately gips-agnostic so the schemas can be exercised in
//! tests without depending on the real IPC stack. The `crony` crate
//! adapts whichever transport gips ships to these traits.

pub mod capabilities;
pub mod control;
pub mod dispatch;
pub mod envelope;
pub mod events;
pub mod session;
pub mod transport;
pub mod version;

pub use capabilities::{CapabilityError, CapabilityRequest, CapabilityResponse};
pub use control::{ControlError, ControlRequest, ControlResponse};
pub use envelope::{
    Channel, ClientToRuntime, CorrelationId, RuntimeToClient, SubscriptionId,
};
pub use events::RuntimeEvent;
pub use transport::{Transport, TransportError};
pub use version::{ProtocolVersion, PROTOCOL_VERSION};
