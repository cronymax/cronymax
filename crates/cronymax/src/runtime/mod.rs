//! Runtime authority — the single source of truth for runs, agents,
//! memory namespaces, and permission state across the Rust runtime.
//!
//! ## Persistence contract (tasks 7.1, 7.2)
//!
//! The runtime owns operational persistence for **all** semantic
//! state: runs, run history (events), memory namespaces, permission
//! reviews and grants, and agent definitions. The on-disk
//! representation is a single versioned [`state::Snapshot`] document
//! written through the [`persistence::Persistence`] trait. See
//! [`state::SNAPSHOT_SCHEMA_VERSION`] and [`state::migrate_snapshot`]
//! for the migration story (task 7.4).
//!
//! The C++ host MUST NOT persist any of the above. Host-owned
//! storage is now restricted to **shell or UI metadata** —
//! window/tab layout, panel state, file pickers, and the like. UI
//! surfaces that previously read from host trace tables now consume
//! [`authority::RuntimeAuthority::run_history`] and the runtime event
//! stream (task 7.3).
//!
//! Submodules:
//!
//! * [`state`] — owned data models (Space, Run, Agent, MemoryNamespace,
//!   PendingReview, plus their statuses). No I/O here.
//! * [`persistence`] — JSON snapshot save/load for crash and restart
//!   rehydration (task 4.4) plus schema migration (task 7.4).
//! * [`authority`] — `RuntimeAuthority`: in-memory state, lifecycle
//!   operations, event fan-out, persistence wiring (tasks 4.2, 4.3,
//!   7.3).
//! * [`handler`] — `RuntimeHandler` adapter from the dispatch
//!   `Handler` trait to `RuntimeAuthority`, replacing `EchoHandler`.
//!
//! Everything in this module is C-FFI-less. The `crony` crate
//! constructs a `RuntimeAuthority` and hands it to the dispatch loop;
//! the rest of the runtime is pure Rust.

pub mod authority;
pub mod chat_store;
pub mod handler;
pub mod middleware;
pub mod persistence;
pub mod prompt;
pub mod sessions;
pub mod state;

pub use authority::{
    AuthorityError, ReviewHandle, ReviewResolution, RuntimeAuthority, SubscribeOutcome,
};
pub use handler::RuntimeHandler;
pub use persistence::{InMemoryPersistence, JsonFilePersistence, Persistence, PersistenceError};
pub use state::{
    Agent, AgentId, HistoryEntry, MemoryEntry, MemoryNamespace, MemoryNamespaceId, PendingReview,
    PermissionState, ReviewId, Run, RunId, RunStatus, Session, SessionId, Snapshot, Space, SpaceId,
};
