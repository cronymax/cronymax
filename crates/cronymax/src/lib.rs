//! `cronymax` — the Rust runtime authority.
//!
//! Responsibilities (filled in by later tasks, see
//! `openspec/changes/rust-runtime-migration/tasks.md`):
//!
//! * Owns runs, agents, memory, permissions, and persistence.
//! * Exposes a control / events / capabilities surface over GIPS.
//! * Composes orchestration primitives from `cronygraph`.
//! * Is **C-FFI-less**: any FFI/CEF integration belongs in the
//!   `crony` crate. This crate must be embeddable from any Rust host
//!   (tests, future tooling, alternative shells).
//!
//! Today this is scaffolding only: a runtime config contract, a
//! protocol-version constant, and a placeholder `Runtime` handle that
//! `crony` can construct/start/stop.

#![forbid(unsafe_code)]

pub mod agent_loop;
pub mod capability;
pub mod config;
pub mod document;
pub mod flow;
pub mod lifecycle;
pub mod llm;
pub mod protocol;
pub mod runtime;
pub mod sandbox;
pub mod terminal;
pub mod workspace;

pub use config::{LogConfig, RuntimeConfig, StoragePaths};
pub use lifecycle::{Runtime, RuntimeError, RuntimeHandle};
pub use protocol::{ProtocolVersion, PROTOCOL_VERSION};
pub use runtime::{
    Agent, AgentId, AuthorityError, InMemoryPersistence, JsonFilePersistence,
    MemoryEntry, MemoryNamespace, MemoryNamespaceId, PendingReview, Persistence,
    PermissionState, ReviewId, Run, RunId, RunStatus, RuntimeAuthority,
    RuntimeHandler, Snapshot, Space, SpaceId,
};

/// Cronymax crate version surfaced for diagnostics and handshakes.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
